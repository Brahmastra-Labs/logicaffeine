//! `ChoreoOp` → C++: every op is one statement kind of the TT-Metalium device API. The kernel text is a
//! function of the class's representative program, the circular-buffer plan and the argument layout; site
//! data enters only through `get_arg_val`.

use std::cell::RefCell;

use super::args::{item_mode, ArgSlot, ArgSource, Axis, ItemMode};
use super::cb::CbPlan;
use super::host::ident;
use super::roles::KernelClass;
use super::SpatialLowerError;
use crate::spatial::ir::*;
use crate::spatial::passes::{LV_ITEM, LV_ITEM_X, LV_ITEM_Y, LV_KBLOCK, LV_SUB_H, LV_SUB_W};
use crate::spatial::target::SpatialTarget;

struct Emitter {
    lines: Vec<String>,
    indent: usize,
}

impl Emitter {
    fn new() -> Emitter {
        Emitter {
            lines: Vec::new(),
            indent: 0,
        }
    }
    fn line(&mut self, s: impl AsRef<str>) {
        let s = s.as_ref();
        if s.starts_with('#') {
            self.lines.push(s.to_string());
        } else {
            self.lines
                .push(format!("{}{}", "    ".repeat(self.indent), s));
        }
    }
    fn open(&mut self, s: impl AsRef<str>) {
        self.line(format!("{} {{", s.as_ref()));
        self.indent += 1;
    }
    /// A bare `{` block (scoping a tile-walk's locals).
    fn block(&mut self) {
        self.line("{");
        self.indent += 1;
    }
    fn close(&mut self) {
        self.indent -= 1;
        self.line("}");
    }
    fn text(self) -> String {
        let mut t = self.lines.join("\n");
        t.push('\n');
        t
    }
}

fn var_name(v: LoopVar) -> &'static str {
    match v {
        x if x == LV_ITEM_Y => "oy",
        x if x == LV_ITEM_X => "ox",
        x if x == LV_KBLOCK => "kb",
        x if x == LV_SUB_H => "sh",
        x if x == LV_SUB_W => "sw",
        x if x == LV_ITEM => "item",
        _ => "lv",
    }
}

/// A tile address as C: loop variables in scope contribute their step; a variable whose loop is absent
/// (a single-iteration sub-block or block loop the schedule elided) contributes index 0, exactly as the
/// evaluator resolves an unbound loop variable.
/// `oy * S + ox` where `S` is the item divisor (`item_id = oy * S + ox` by construction) is `item_id`
/// itself: a linear per-tile output is written by its item id, as the vendor writer does.
fn item_identity(ctx: &Ctx, tensor: TensorId, at: &TileAddr, scope: &[LoopVar]) -> bool {
    if ctx.mode != ItemMode::Linear || at.base != 0 {
        return false;
    }
    // The steps that contribute: bound loop variables (item coordinates always are) with a non-zero stride.
    let live: Vec<(LoopVar, usize)> = at
        .steps
        .iter()
        .copied()
        .filter(|(v, s)| *s != 0 && (scope.contains(v) || *v == LV_ITEM_Y || *v == LV_ITEM_X))
        .collect();
    if live.len() != 2 {
        return false;
    }
    let sy = live.iter().find(|(v, _)| *v == LV_ITEM_Y).map(|(_, s)| *s);
    let sx = live.iter().find(|(v, _)| *v == LV_ITEM_X).map(|(_, s)| *s);
    match (sy, sx, ctx.p.output_tensor()) {
        (Some(sy), Some(1), Some(out)) => {
            ctx.stride_expr(tensor, sy) == ctx.count_expr(out.id, Axis::X, ctx.grid_nx)
        }
        _ => false,
    }
}

fn addr_expr(ctx: &Ctx, tensor: TensorId, at: &TileAddr, scope: &[LoopVar]) -> String {
    if item_identity(ctx, tensor, at, scope) {
        return "item_id".into();
    }
    let mut terms: Vec<String> = Vec::new();
    if at.base != 0 {
        terms.push(ctx.stride_expr(tensor, at.base));
    }
    for (v, stride) in &at.steps {
        let bound = scope.contains(v) || *v == LV_ITEM_Y || *v == LV_ITEM_X;
        if !bound || *stride == 0 {
            continue;
        }
        let s = ctx.stride_expr(tensor, *stride);
        terms.push(if s == "1u" {
            var_name(*v).to_string()
        } else if s.contains(' ') {
            format!("{} * ({s})", var_name(*v))
        } else {
            format!("{} * {s}", var_name(*v))
        });
    }
    if terms.is_empty() {
        "0u".into()
    } else {
        terms.join(" + ")
    }
}

fn sem_value(v: SemValue) -> String {
    match v {
        SemValue::Invalid => "INVALID".into(),
        SemValue::Valid => "VALID".into(),
        SemValue::Count(n) => format!("{n}u"),
    }
}

struct Ctx<'a> {
    p: &'a SpatialProgram,
    cbs: &'a CbPlan,
    slots: &'a [ArgSlot],
    mode: ItemMode,
    grid_nx: usize,
    stride_x: usize,
    stride_y: usize,
    /// Buffer templates this program fills through a write pointer (tile reads) …
    written: Vec<BufferId>,
    /// … and those it multicasts from (need the block start address).
    multicast: Vec<BufferId>,
    /// Movement kernels take shape numbers as runtime parameters; compute kernels fold them.
    params_as_args: bool,
    /// Distinct data formats this program packs into: more than one means the packer is reconfigured per pack.
    pack_formats: usize,
    /// The schedule uses the packer's L1 accumulate anywhere, so every pack states its mode.
    pack_accumulates: bool,
    /// The packer's live `(destination format, accumulate)` while printing, so a reconfiguration is emitted
    /// only where the state changes. `None` means unknown — at kernel entry and on entering a loop body.
    pack_state: RefCell<Option<(String, bool)>>,
    params: RefCell<Vec<ArgSlot>>,
    site_slots: usize,
}

impl Ctx<'_> {
    /// A structural number (a block extent, a page count): always a literal, so single-tile block loops
    /// vanish at compile time exactly as the vendor's do.
    fn lit(&self, value: usize) -> String {
        format!("{value}u")
    }

    /// A tensor's tile-grid extent: the literal in a compute kernel; in a movement kernel the runtime
    /// parameter (one slot per tensor axis, created on first use) through which shape — and only shape —
    /// enters the kernel.
    fn dim(&self, tensor: TensorId, axis: Axis) -> String {
        let tiles = self.p.tensor(tensor).tiles;
        let value = match axis {
            Axis::Y => tiles.rows_t,
            Axis::X => tiles.cols_t,
        };
        if !self.params_as_args {
            return format!("{value}u");
        }
        // One symbol per equal-extent class (a GEMM's N is B's width AND C's width): the address arithmetic
        // names each extent once, and the compiler keeps one register for it.
        let (rep, rep_axis) = self.p.dim_representative(
            tensor,
            match axis {
                Axis::Y => DimAxis::Rows,
                Axis::X => DimAxis::Cols,
            },
        );
        let axis = match rep_axis {
            DimAxis::Rows => Axis::Y,
            DimAxis::Cols => Axis::X,
        };
        let name = format!(
            "p_{}_{}",
            ident(&self.p.tensor(rep).name),
            match axis {
                Axis::Y => "rows",
                Axis::X => "cols",
            }
        );
        let mut params = self.params.borrow_mut();
        if !params.iter().any(|s| s.name == name) {
            params.push(ArgSlot {
                name: name.clone(),
                source: ArgSource::TensorDim { tensor: rep, axis },
            });
        }
        name
    }

    /// A tile stride or base offset into `tensor` (row-major tiles): `q · cols_t + r` printed as
    /// `q * p_cols + r` with `q`, `r` block-extent literals — so a unit or block-only stride stays a literal
    /// (the compiler folds it) and a row stride rides the tensor's runtime width, exactly the vendor's
    /// `out_row * Kt + k` form.
    fn stride_expr(&self, tensor: TensorId, value: usize) -> String {
        if !self.params_as_args {
            return format!("{value}u");
        }
        let cols = self.p.tensor(tensor).tiles.cols_t;
        if value == 0 || cols <= 1 || value < cols {
            return format!("{value}u");
        }
        let (q, r) = (value / cols, value % cols);
        let width = self.dim(tensor, Axis::X);
        let main = if q == 1 {
            width
        } else {
            format!("{q}u * {width}")
        };
        if r == 0 {
            main
        } else {
            format!("{main} + {r}u")
        }
    }

    /// A loop count or item divisor that is a tensor extent over a block extent (`Kt / ibw`, `Nt / per_site_n`):
    /// `p_dim / q` with `q` a literal — a single-iteration count (`value == 1`) stays the literal `1u` so the
    /// loop vanishes at compile time like the vendor's.
    fn count_expr(&self, tensor: TensorId, axis: Axis, value: usize) -> String {
        if !self.params_as_args || value <= 1 {
            return format!("{value}u");
        }
        let tiles = self.p.tensor(tensor).tiles;
        let dim = match axis {
            Axis::Y => tiles.rows_t,
            Axis::X => tiles.cols_t,
        };
        if dim == 0 || dim % value != 0 {
            return format!("{value}u");
        }
        let q = dim / value;
        let name = self.dim(tensor, axis);
        if q == 1 {
            name
        } else {
            format!("({name} / {q}u)")
        }
    }

    /// The tensor whose width is the reduction extent (the first input whose tile columns count K).
    fn k_tensor(&self) -> Option<TensorId> {
        self.p
            .tensors
            .iter()
            .find(|t| t.role != TensorRole::Output && t.tiles.cols_t == self.p.work.k_tiles)
            .map(|t| t.id)
    }
    fn tile_bytes(&self, b: BufferId) -> String {
        let cb = self.cb(b);
        if self.params_as_args {
            format!("tile_bytes_{cb}")
        } else {
            format!("TILE_BYTES_{cb}")
        }
    }
    fn arg(&self, source: &ArgSource) -> String {
        self.slots
            .iter()
            .find(|s| &s.source == source)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| panic!("no slot for {source:?}"))
    }
    /// The circular buffer a compute op packs into. `cb.rs` fixes the index classes — operand
    /// inputs `c_0..`, outputs `c_16..`, spills `c_24..` — so the output is the slot in `16..24`.
    /// `matmul_init` does not need it, which is why nothing asked for it before; every binary and
    /// SFPU init on Tensix takes the destination.
    fn out_cb(&self) -> Result<String, SpatialLowerError> {
        self.cbs
            .slots
            .iter()
            .find(|s| (16..24).contains(&s.index))
            .map(|s| s.c_name())
            .ok_or(SpatialLowerError::Unsupported(
                "a compute program with no output circular buffer",
            ))
    }

    fn cb(&self, b: BufferId) -> String {
        self.cbs.slot(b).c_name()
    }
    /// The circular buffer's page: the template's, which every site of the template reserves, pushes, waits
    /// and pops in (so each reservation divides the buffer).
    fn page_tiles(&self, b: BufferId) -> usize {
        self.cbs.slot(b).page_tiles
    }
    /// The tiles one page of THIS site's instance of `b` holds — what its multicast puts on the NOC. A
    /// boundary site's page is narrower than the template's, and sending the template's would move rows that
    /// do not exist; every site of one kernel shares it (`SpatialProgram::engine_signature` keys a buffer by
    /// its page).
    fn sent_tiles(&self, b: BufferId) -> usize {
        self.p.buffer(b).kind.page_tiles()
    }
    fn operand_of_group(&self, g: GroupId) -> Operand {
        self.p.placement.as_ref().unwrap().groups[g.idx()].operand
    }
    fn operand_of_sem(&self, s: SemId) -> Operand {
        self.p.semaphores[s.idx()]
            .operand
            .expect("a multicast semaphore")
    }
    fn mcast_rect_args(&self, operand: Operand) -> (String, String, String, String, String) {
        use super::args::Axis;
        (
            self.arg(&ArgSource::McastStart {
                operand,
                axis: Axis::X,
            }),
            self.arg(&ArgSource::McastStart {
                operand,
                axis: Axis::Y,
            }),
            self.arg(&ArgSource::McastEnd {
                operand,
                axis: Axis::X,
            }),
            self.arg(&ArgSource::McastEnd {
                operand,
                axis: Axis::Y,
            }),
            self.arg(&ArgSource::McastNumDests { operand }),
        )
    }
    /// The `oy`/`ox` derivation for the current item (`item` in an item loop, the first item otherwise).
    /// The item id (a linear enumeration) and, when an address needs them, its `oy`/`ox` coordinates.
    fn coord_lines(&self, e: &mut Emitter, in_loop: bool, coords: bool) {
        let idx = if in_loop { "item" } else { "0u" };
        match self.mode {
            ItemMode::Linear => {
                e.line(format!(
                    "const uint32_t item_id = {} + {idx};",
                    self.arg(&ArgSource::ItemStart)
                ));
                if coords {
                    let nx = match self.p.output_tensor() {
                        Some(out) => self.count_expr(out.id, Axis::X, self.grid_nx),
                        None => self.lit(self.grid_nx),
                    };
                    e.line(format!("const uint32_t oy = item_id / {nx};"));
                    e.line(format!("const uint32_t ox = item_id % {nx};"));
                }
            }
            ItemMode::Strided => {
                let cx = self.arg(&ArgSource::ItemCountX);
                let (sy, sx) = (self.lit(self.stride_y), self.lit(self.stride_x));
                e.line(format!(
                    "const uint32_t oy = {} + ({idx} / {cx}) * {sy};",
                    self.arg(&ArgSource::ItemY0)
                ));
                e.line(format!(
                    "const uint32_t ox = {} + ({idx} % {cx}) * {sx};",
                    self.arg(&ArgSource::ItemX0)
                ));
            }
        }
    }
}

/// `(needs the item id, needs oy/ox)` for the addresses in `ops` (an address inside an elided loop sees the
/// loop variables of `scope` plus those of the loops that wrap it in `ops`).
fn uses_coords(ctx: &Ctx, ops: &[ChoreoOp], scope: &[LoopVar]) -> (bool, bool) {
    let (mut item, mut coords) = (false, false);
    let mut inner_scope: Vec<LoopVar> = scope.to_vec();
    walk_ops(ops, 1, &mut |op, _| {
        if let ChoreoOp::Loop { var, .. } = op {
            if !inner_scope.contains(var) {
                inner_scope.push(*var);
            }
        }
    });
    walk_ops(ops, 1, &mut |op, _| {
        if let ChoreoOp::ReadTiles { tensor, at, .. } | ChoreoOp::WriteTiles { tensor, at, .. } = op
        {
            if at
                .steps
                .iter()
                .any(|(v, _)| *v == LV_ITEM_Y || *v == LV_ITEM_X)
            {
                if item_identity(ctx, *tensor, at, &inner_scope) {
                    item = true;
                } else {
                    coords = true;
                }
            }
        }
    });
    (item || coords, coords)
}

fn emit_ops(
    ctx: &Ctx,
    e: &mut Emitter,
    ops: &[ChoreoOp],
    compute: bool,
    scope: &mut Vec<LoopVar>,
) -> Result<(), SpatialLowerError> {
    for (i, op) in ops.iter().enumerate() {
        match op {
            ChoreoOp::Reserve { buf, pages } => {
                let cb = ctx.cb(*buf);
                // A compute engine reserving the page it is about to pack: the math is committed and waited
                // for BEFORE the reservation (the vendor's order — the packer reserves once the tile is ready).
                let packs_next = compute && matches!(ops.get(i + 1), Some(ChoreoOp::PackAcc { to, .. }) if to == buf);
                if packs_next {
                    e.line("tile_regs_commit();");
                    e.line("tile_regs_wait();");
                }
                e.line(format!("cb_reserve_back({cb}, {});", ctx.lit(pages * ctx.page_tiles(*buf))));
                let template = ctx.cbs.slot(*buf).template;
                if !compute && ctx.written.contains(&template) {
                    e.line(format!("wp_{cb} = get_write_ptr({cb});"));
                    if ctx.multicast.contains(&template) {
                        e.line(format!("wp_{cb}_start = wp_{cb};"));
                    }
                }
            }
            ChoreoOp::Push { buf, pages } => e.line(format!("cb_push_back({}, {});", ctx.cb(*buf), ctx.lit(pages * ctx.page_tiles(*buf)))),
            ChoreoOp::Wait { buf, pages } => e.line(format!("cb_wait_front({}, {});", ctx.cb(*buf), ctx.lit(pages * ctx.page_tiles(*buf)))),
            ChoreoOp::Pop { buf, pages } => e.line(format!("cb_pop_front({}, {});", ctx.cb(*buf), ctx.lit(pages * ctx.page_tiles(*buf)))),
            ChoreoOp::ReadTiles { tensor, at, block_h, block_w, stride_w, stride_h, into } => {
                let cb = ctx.cb(*into);
                if *block_h == 1 && *block_w == 1 {
                    // A one-tile block is one page: the read, nothing else.
                    e.line(format!("noc_async_read_page({}, acc_t{}, wp_{cb});", addr_expr(ctx, *tensor, at, scope), tensor.idx()));
                } else {
                    e.block();
                    e.line(format!("uint32_t row_id = {};", addr_expr(ctx, *tensor, at, scope)));
                    e.open(format!("for (uint32_t h = 0; h < {}; ++h)", ctx.lit(*block_h)));
                    e.line("uint32_t tile_id = row_id;");
                    e.open(format!("for (uint32_t w = 0; w < {}; ++w)", ctx.lit(*block_w)));
                    e.line(format!("noc_async_read_page(tile_id, acc_t{}, wp_{cb});", tensor.idx()));
                    e.line(format!("wp_{cb} += {};", ctx.tile_bytes(*into)));
                    e.line(format!("tile_id += {};", ctx.stride_expr(*tensor, *stride_w)));
                    e.close();
                    e.line(format!("row_id += {};", ctx.stride_expr(*tensor, *stride_h)));
                    e.close();
                    e.close();
                }
            }
            ChoreoOp::ReadBarrier => e.line("noc_async_read_barrier();"),
            ChoreoOp::WriteTiles { from, tensor, at, block_h, block_w, stride_w, stride_h } => {
                let cb = ctx.cb(*from);
                if *block_h == 1 && *block_w == 1 {
                    e.line(format!("noc_async_write_page({}, acc_t{}, get_read_ptr({cb}));", addr_expr(ctx, *tensor, at, scope), tensor.idx()));
                } else {
                    e.block();
                    e.line(format!("uint32_t rp = get_read_ptr({cb});"));
                    e.line(format!("uint32_t row_id = {};", addr_expr(ctx, *tensor, at, scope)));
                    e.open(format!("for (uint32_t h = 0; h < {}; ++h)", ctx.lit(*block_h)));
                    e.line("uint32_t tile_id = row_id;");
                    e.open(format!("for (uint32_t w = 0; w < {}; ++w)", ctx.lit(*block_w)));
                    e.line(format!("noc_async_write_page(tile_id, acc_t{}, rp);", tensor.idx()));
                    e.line(format!("rp += {};", ctx.tile_bytes(*from)));
                    e.line(format!("tile_id += {};", ctx.stride_expr(*tensor, *stride_w)));
                    e.close();
                    e.line(format!("row_id += {};", ctx.stride_expr(*tensor, *stride_h)));
                    e.close();
                    e.close();
                }
            }
            ChoreoOp::WriteBarrier => e.line("noc_async_write_barrier();"),
            ChoreoOp::McastSend { buf, pages, group, hold_path } => {
                let cb = ctx.cb(*buf);
                let (sx, sy, ex, ey, nd) = ctx.mcast_rect_args(ctx.operand_of_group(*group));
                e.line(format!(
                    "noc_async_write_multicast(wp_{cb}_start, get_noc_multicast_addr({sx}, {sy}, {ex}, {ey}, wp_{cb}_start), {} * {}, {nd}, {});",
                    ctx.lit(pages * ctx.sent_tiles(*buf)),
                    ctx.tile_bytes(*buf),
                    hold_path
                ));
            }
            ChoreoOp::McastSetValid { sem, group } => {
                let (sx, sy, ex, ey, nd) = ctx.mcast_rect_args(ctx.operand_of_group(*group));
                let s = sem.idx();
                e.line(format!("noc_semaphore_set_multicast(sem{s}_addr, get_noc_multicast_addr({sx}, {sy}, {ex}, {ey}, sem{s}_addr), {nd});"));
            }
            ChoreoOp::Flush { cond } => match cond {
                ArchCond::Blackhole => {
                    e.line("#ifdef ARCH_BLACKHOLE");
                    e.line("noc_async_writes_flushed();");
                    e.line("#endif");
                }
                ArchCond::Always => e.line("noc_async_writes_flushed();"),
            },
            ChoreoOp::SemSet { sem, value } => e.line(format!("noc_semaphore_set(sem{}_ptr, {});", sem.idx(), sem_value(*value))),
            ChoreoOp::SemInc { sem, by, .. } => {
                use super::args::Axis;
                let operand = ctx.operand_of_sem(*sem);
                let sx = ctx.arg(&ArgSource::McastSender { operand, axis: Axis::X });
                let sy = ctx.arg(&ArgSource::McastSender { operand, axis: Axis::Y });
                e.line(format!("noc_semaphore_inc(get_noc_addr({sx}, {sy}, sem{}_addr), {by}u);", sem.idx()));
            }
            ChoreoOp::SemWait { sem, until } => match until {
                SemCond::Equals(n) => e.line(format!("noc_semaphore_wait(sem{}_ptr, {});", sem.idx(), if *n == 1 { "VALID".to_string() } else { format!("{n}u") })),
                SemCond::AtLeast(n) => e.line(format!("noc_semaphore_wait_min(sem{}_ptr, {n}u);", sem.idx())),
            },
            ChoreoOp::AcquireAcc { .. } => e.line("tile_regs_acquire();"),
            ChoreoOp::ReleaseAcc => e.line("tile_regs_release();"),
            ChoreoOp::Compute { op, ins, acc_reload } => emit_compute(ctx, e, op, ins, *acc_reload)?,
            ChoreoOp::PackAcc { to, tiles, accumulate, .. } => {
                // The packer holds its destination format and its accumulate mode; each reconfiguration
                // stalls the pack pipe, so one is emitted only where the schedule CHANGES the state. A loop
                // body is entered from two places (fall-through and back-edge), so a state set inside one is
                // not carried out of it.
                let want = (ctx.cb(*to), *accumulate);
                let mut pack = ctx.pack_state.borrow_mut();
                if ctx.pack_formats > 1 && pack.as_ref().map(|(f, _)| f != &want.0).unwrap_or(true) {
                    e.line(format!("pack_reconfig_data_format({});", want.0));
                }
                if ctx.pack_accumulates && pack.as_ref().map(|(_, a)| *a != want.1).unwrap_or(true) {
                    e.line(format!("pack_reconfig_l1_acc({});", want.1 as u8));
                }
                *pack = Some(want);
                drop(pack);
                let reserved_just_before = matches!(ops.get(i.wrapping_sub(1)), Some(ChoreoOp::Reserve { buf, .. }) if buf == to) && i > 0;
                if !reserved_just_before {
                    e.line("tile_regs_commit();");
                    e.line("tile_regs_wait();");
                }
                let cb = ctx.cb(*to);
                e.open(format!("for (uint32_t i = 0; i < {tiles}u; ++i)"));
                e.line(format!("pack_tile(i, {cb});"));
                e.close();
            }
            ChoreoOp::Barrier { .. } => return Err(SpatialLowerError::Unsupported("a group barrier on Tensix")),
            ChoreoOp::Loop { count: LoopCount::Trip { .. }, .. } => {
                return Err(SpatialLowerError::Unsupported("a loop whose count the program reads from its data (a trip) — Tensix kernel loops are bounded by compile-time constants and item-count runtime arguments"))
            }
            ChoreoOp::Loop { var, count, body } => {
                let v = var_name(*var);
                let bound = if *var == LV_ITEM {
                    ctx.arg(&ArgSource::ItemCount)
                } else if *var == LV_KBLOCK {
                    match ctx.k_tensor() {
                        Some(t) => ctx.count_expr(t, Axis::X, count.bound()),
                        None => ctx.lit(count.bound()),
                    }
                } else {
                    ctx.lit(count.bound())
                };
                let inner = body_ops(body);
                *ctx.pack_state.borrow_mut() = None;
                e.open(format!("for (uint32_t {v} = 0; {v} < {bound}; ++{v})"));
                if *var == LV_ITEM {
                    let (item, coords) = uses_coords(ctx, &inner, scope);
                    if item {
                        ctx.coord_lines(e, true, coords);
                    }
                }
                scope.push(*var);
                emit_ops(ctx, e, &inner, compute, scope)?;
                scope.pop();
                *ctx.pack_state.borrow_mut() = None;
                e.close();
            }
        }
    }
    Ok(())
}

/// The compute-API headers a kernel's ops actually need, beyond the base set every compute kernel gets.
fn compute_includes(ops: &[ChoreoOp]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let need = |h: &'static str, out: &mut Vec<&'static str>| {
        if !out.contains(&h) {
            out.push(h);
        }
    };
    walk_ops(ops, 1, &mut |op, _| {
        if let ChoreoOp::Compute { op, .. } = op {
            match op {
                ComputeOp::Generic { iters, body, .. } => {
                    need("api/compute/eltwise_binary.h", &mut out);
                    need("api/compute/bcast.h", &mut out);
                    if iters
                        .get(1)
                        .map(|a| matches!(a.kind, IterKind::Reduce { .. }))
                        .unwrap_or(false)
                    {
                        need("api/compute/reduce.h", &mut out);
                    }
                    let sfpu = |k: &SfpuKind, out: &mut Vec<&'static str>| match k {
                        SfpuKind::Exp => need("api/compute/eltwise_unary/exp.h", out),
                        SfpuKind::Recip => need("api/compute/eltwise_unary/recip.h", out),
                        SfpuKind::Rsqrt { .. } => {
                            need("api/compute/eltwise_unary/rsqrt.h", out);
                            need("api/compute/eltwise_unary/binop_with_scalar.h", out);
                        }
                        SfpuKind::Silu | SfpuKind::Relu | SfpuKind::Sign => {
                            need("api/compute/compute_kernel_api.h", out)
                        }
                        // Refused by `emit_sfpu`; there is no header to pull for it.
                        SfpuKind::Ln => {}
                    };
                    // ★ WALK THE WHOLE BODY, NOT ITS TOP NODE. Fusion nests transforms inside
                    // arithmetic — `Mul(Silu(x), u)` — so inspecting only the outermost node missed
                    // `silu_tile` and the kernel failed to COMPILE under tt-metal, which is the one
                    // place a missing header is expensive to read. A recursive walk cannot miss one.
                    fn sfpu_kinds(b: &TensorBody, acc: &mut Vec<SfpuKind>) {
                        match b {
                            TensorBody::Unary { kind, arg } => {
                                acc.push(*kind);
                                sfpu_kinds(arg, acc);
                            }
                            TensorBody::Add(x, y)
                            | TensorBody::Sub(x, y)
                            | TensorBody::Mul(x, y)
                            | TensorBody::Min(x, y)
                            | TensorBody::Max(x, y)
                            | TensorBody::FloorDiv(x, y) => {
                                sfpu_kinds(x, acc);
                                sfpu_kinds(y, acc);
                            }
                            TensorBody::Shr { arg, .. } => sfpu_kinds(arg, acc),
                            TensorBody::Select {
                                then, otherwise, ..
                            } => {
                                sfpu_kinds(then, acc);
                                sfpu_kinds(otherwise, acc);
                            }
                            TensorBody::Operand(_) | TensorBody::Const(_) => {}
                        }
                    }
                    let mut kinds = Vec::new();
                    sfpu_kinds(body, &mut kinds);
                    for k in &kinds {
                        sfpu(k, &mut out);
                    }
                    fn has_const(b: &TensorBody) -> bool {
                        match b {
                            TensorBody::Const(_) => true,
                            TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => {
                                has_const(arg)
                            }
                            TensorBody::Add(x, y)
                            | TensorBody::Sub(x, y)
                            | TensorBody::Mul(x, y)
                            | TensorBody::Min(x, y)
                            | TensorBody::Max(x, y)
                            | TensorBody::FloorDiv(x, y) => has_const(x) || has_const(y),
                            TensorBody::Select {
                                then, otherwise, ..
                            } => has_const(then) || has_const(otherwise),
                            TensorBody::Operand(_) => false,
                        }
                    }
                    if has_const(body) {
                        need("api/compute/eltwise_unary/binop_with_scalar.h", &mut out);
                        need("api/compute/tile_move_copy.h", &mut out);
                    }
                    if !kinds.is_empty() {
                        // `copy_tile` feeds every SFPU sequence.
                        need("api/compute/tile_move_copy.h", &mut out);
                    }
                }
                _ => {}
            }
        }
    });
    out
}

/// `x ⊕ <literal>` — the SFPU's scalar binop, which takes the constant as a float bit pattern.
fn emit_scalar(
    e: &mut Emitter,
    stem: &str,
    bits: u64,
    tiles: usize,
) -> Result<(), SpatialLowerError> {
    let v = f64::from_bits(bits) as f32;
    let call = match stem {
        "add" => "add_unary_tile",
        "sub" => "sub_unary_tile",
        "mul" => "mul_unary_tile",
        other => {
            return Err(SpatialLowerError::Unsupported(match other {
                _ => "a scalar body operation with no SFPU spelling on Tensix",
            }))
        }
    };
    e.line("binop_with_scalar_tile_init();".to_string());
    e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
    e.line(format!(
        "{call}(t, 0x{:08x}u);  // {stem} {v:e}",
        v.to_bits()
    ));
    e.close();
    Ok(())
}

/// Emit a body whose result is left in the destination registers. Used for the INNER half of a
/// composed body, where the outer operation then consumes DST via its dest-reuse form.
#[allow(clippy::too_many_arguments)]
fn emit_body(
    ctx: &Ctx,
    e: &mut Emitter,
    body: &TensorBody,
    ins: &[BufferId],
    maps: &[ElemAccess],
    tiles: usize,
    row_ax: IterSpec,
    col_ax: IterSpec,
    out_cb: &str,
) -> Result<(), SpatialLowerError> {
    match body {
        TensorBody::Operand(i) => {
            let x = ctx.cb(*ins.get(*i).ok_or(SpatialLowerError::Unsupported(
                "a Generic body naming an absent operand",
            ))?);
            e.line(format!("copy_tile_to_dst_init_short({x});"));
            e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
            e.line(format!("copy_tile({x}, t, t);"));
            e.close();
            Ok(())
        }
        TensorBody::Unary { kind, arg } => {
            emit_body(ctx, e, arg, ins, maps, tiles, row_ax, col_ax, out_cb)?;
            emit_sfpu(e, *kind, tiles)
        }
        TensorBody::Min(..) | TensorBody::Max(..) | TensorBody::FloorDiv(..) | TensorBody::Shr { .. } => {
            Err(SpatialLowerError::Unsupported(
                "an integer model's min, max, floor division or arithmetic shift on Tensix — its compute kernel is float-primary and prints none of them",
            ))
        }
        TensorBody::Add(a, b) | TensorBody::Sub(a, b) | TensorBody::Mul(a, b)
            if matches!(**b, TensorBody::Const(_)) =>
        {
            let TensorBody::Const(c) = **b else {
                unreachable!()
            };
            emit_body(ctx, e, a, ins, maps, tiles, row_ax, col_ax, out_cb)?;
            let stem = match body {
                TensorBody::Add(..) => "add",
                TensorBody::Sub(..) => "sub",
                _ => "mul",
            };
            emit_scalar(e, stem, c, tiles)
        }
        TensorBody::Add(a, b) | TensorBody::Sub(a, b) | TensorBody::Mul(a, b) => {
            let (TensorBody::Operand(ia), TensorBody::Operand(ib)) = (&**a, &**b) else {
                return Err(SpatialLowerError::Unsupported(
                    "a Generic body nested more than one level on Tensix",
                ));
            };
            // ★ A BROADCAST IS FINE HERE — IT IS THE OUTER OPERATION THAT CANNOT DO ONE. This is the
            // INNERMOST operation, so it PRODUCES the destination registers from two circular
            // buffers, which is exactly what the `_bcast_` family does. Only dest-REUSE (the outer
            // half of a composition) has no broadcast spelling. Refusing here made softmax's fused
            // `exp(x - rowmax)` unlowerable for a restriction that belongs one level up.
            let stem = match body {
                TensorBody::Add(..) => "add",
                TensorBody::Sub(..) => "sub",
                _ => "mul",
            };
            let (ca, cbb) = (ctx.cb(ins[*ia]), ctx.cb(ins[*ib]));
            let axis = match maps
                .get(*ib)
                .map(|m| (m.varies_along(row_ax.var), m.varies_along(col_ax.var)))
            {
                Some((true, true)) | None => "",
                Some((true, false)) => "_bcast_cols",
                Some((false, true)) => "_bcast_rows",
                Some((false, false)) => "_bcast_scalar",
            };
            e.line(format!("binary_op_init_common({ca}, {cbb}, {out_cb});"));
            if axis.is_empty() {
                e.line(format!("{stem}_tiles_init({ca}, {cbb});"));
                e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
                e.line(format!("{stem}_tiles({ca}, {cbb}, t, t, t);"));
            } else {
                e.line(format!("{stem}{axis}_init({ca}, {cbb});"));
                e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
                let src = if axis == "_bcast_cols" { "0" } else { "t" };
                e.line(format!("{stem}_tiles{axis}({ca}, {cbb}, t, {src}, t);"));
            }
            e.close();
            Ok(())
        }
        TensorBody::Const(_) => Err(SpatialLowerError::Unsupported(
            "a bare constant inside a Generic body on Tensix",
        )),
        TensorBody::Select { .. } => Err(SpatialLowerError::Unsupported(
            "a piecewise Select body on Tensix (no predicated tile sequence is printed)",
        )),
    }
}

/// One SFPU instruction over `tiles` destination registers. A new transform is a new arm HERE — a
/// handful of lines naming a hardware instruction — never a new `ComputeOp`.
fn emit_sfpu(e: &mut Emitter, kind: SfpuKind, tiles: usize) -> Result<(), SpatialLowerError> {
    let (init, call) = match kind {
        SfpuKind::Exp => ("exp_tile_init", "exp_tile"),
        SfpuKind::Recip => ("recip_tile_init", "recip_tile"),
        SfpuKind::Rsqrt { .. } => ("rsqrt_tile_init", "rsqrt_tile"),
        SfpuKind::Silu => ("silu_tile_init", "silu_tile"),
        SfpuKind::Relu => ("relu_tile_init", "relu_tile"),
        SfpuKind::Sign => ("sign_tile_init", "sign_tile"),
        SfpuKind::Ln => {
            return Err(SpatialLowerError::Unsupported(
                "a natural-log SFPU transform on Tensix (no log_tile sequence is printed)",
            ))
        }
    };
    if let SfpuKind::Rsqrt { eps_nano, over } = kind {
        let eps = (eps_nano as f64 * 1e-9) as f32;
        let inv = (1.0f64 / over.max(1) as f64) as f32;
        e.line("binop_with_scalar_tile_init();".to_string());
        e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
        e.line(format!(
            "mul_unary_tile(t, 0x{:08x}u);  // / {over}",
            inv.to_bits()
        ));
        e.line(format!(
            "add_unary_tile(t, 0x{:08x}u);  // + {eps:e}",
            eps.to_bits()
        ));
        e.close();
    }
    e.line(format!("{init}();"));
    e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
    e.line(format!("{call}(t);"));
    e.close();
    Ok(())
}

fn emit_compute(
    ctx: &Ctx,
    e: &mut Emitter,
    op: &ComputeOp,
    ins: &[BufferId],
    reload: Option<BufferId>,
) -> Result<(), SpatialLowerError> {
    match op {
        ComputeOp::MatmulTiles {
            block,
            sub_blocks,
            granularity,
            ..
        } => {
            let (in0, in1) = (ctx.cb(ins[0]), ctx.cb(ins[1]));
            let BlockShape { osh, osw, ibw } = *block;
            let pn = sub_blocks.1 * osw;
            // The engine is configured for the shape it will be handed.
            let init = match granularity {
                Granularity::PerSubBlock => {
                    format!("matmul_block_init({in0}, {in1}, 0, {osw}u, {osh}u, {ibw}u);")
                }
                Granularity::PerTile => format!("matmul_init({in0}, {in1});"),
            };
            if let Some(interm) = reload {
                let cbi = ctx.cb(interm);
                // The unpacker is configured for the matmul operands; a spilled partial in another format
                // (exact fp32 partials under bf16 operands) must reconfigure srcA for the reload — the plain
                // init does not touch data formats — and the matmul init restores the operand formats.
                let reconfigured = ctx.p.buffer(interm).dtype != ctx.p.buffer(ins[1]).dtype;
                if reconfigured {
                    e.line(format!(
                        "copy_tile_to_dst_init_short_with_dt({in1}, {cbi});"
                    ));
                } else {
                    e.line(format!("copy_tile_to_dst_init_short({cbi});"));
                }
                e.open(format!("for (uint32_t i = 0; i < {}u; ++i)", osh * osw));
                e.line(format!("copy_tile({cbi}, i, i);"));
                e.close();
                if reconfigured {
                    // Put source A back to the operand's format: the matmul init does NOT touch data
                    // formats, so an unrestored source A unpacks the next operand as the spill's type.
                    e.line(format!("reconfig_data_format_srca({cbi}, {in1});"));
                }
                e.line(init.clone());
            }
            let (sh, sw) = (
                if sub_blocks.0 > 1 { "sh" } else { "0u" },
                if sub_blocks.1 > 1 { "sw" } else { "0u" },
            );
            match granularity {
                Granularity::PerSubBlock => {
                    // One call per inner-dim step covers the whole `osh × osw` sub-block: the engine strides
                    // A by one tile and B by the block width itself.
                    e.open(format!("for (uint32_t kk = 0; kk < {ibw}u; ++kk)"));
                    e.line(format!(
                        "matmul_block({in0}, {in1}, ({sh} * {osh}u) * {ibw}u + kk, kk * {pn}u + {sw} * {osw}u, 0, 0, {osw}u, {osh}u, {ibw}u);"
                    ));
                    e.close();
                }
                Granularity::PerTile => {
                    e.open(format!("for (uint32_t h = 0; h < {osh}u; ++h)"));
                    e.open(format!("for (uint32_t w = 0; w < {osw}u; ++w)"));
                    e.open(format!("for (uint32_t kk = 0; kk < {ibw}u; ++kk)"));
                    e.line(format!("matmul_tiles({in0}, {in1}, ({sh} * {osh}u + h) * {ibw}u + kk, kk * {pn}u + {sw} * {osw}u + w, h * {osw}u + w);"));
                    e.close();
                    e.close();
                    e.close();
                }
            }
            Ok(())
        }
        // ── P4: the ops the legacy `codegen_tt` templates were the only Tensix path for ──
        //
        // The surrounding choreography owns the handshake: `AcquireAcc`/`ReleaseAcc` bracket this
        // body and `Pack` writes the result out, so an arm emits ONLY the engine configuration and
        // the op itself. That is the whole reason there is one `cb_push_back` in this file and
        // twenty-five in the templates being retired.
        // ── THE GENERAL FORM ───────────────────────────────────────────────────────────────────
        //
        // ★ THE PRINTER READS THE MAPS; IT DOES NOT RECOGNIZE A WORKLOAD. Which broadcast the silicon
        // wants is not a name the front end passed down — it is derived here, from which axis an
        // operand's access map omits. Omit the column axis and the operand is one scalar per row, so
        // the engine wants `_bcast_cols`; omit the row axis and it is one value per column, so it
        // wants `_bcast_rows`; omit neither and it is a plain tile-for-tile op. That derivation is
        // the whole point of the algebra: adding a workload adds no arm here.
        ComputeOp::Generic {
            iters,
            ins: maps,
            out,
            body,
        } => {
            if iters.len() != 2 {
                return Err(SpatialLowerError::Unsupported("a Generic with a contraction axis (LV_GEN_K, a third folded iteration variable) on Tensix — no matrix-engine sequence spelled for it"));
            }
            let (row_ax, col_ax) = (iters[0], iters[1]);
            let out_cb = ctx.out_cb()?;
            // ★ THE TILE IS THE DESCRIPTOR'S, NOT A CONSTANT. A literal 32 here is the machine
            // standing in for itself: the tree already carries TileShape::WMMA16 (16x16) and
            // TileShape::SCALAR (1x1) beside TileShape::TENSIX, and `page_width` — the function that
            // BUILT the strides checked below — reads `target.tile.cols`. Two copies of one rule
            // disagree the first time either moves, and the disagreement here would be a width, which
            // is the quantity this arm refuses on. Read the same rule the builder wrote.
            let tile_w = ctx.p.tile.cols.max(1);
            let tiles = col_ax.extent.div_ceil(tile_w).max(1);
            // How an operand's map relates to the iteration space decides the instruction family.
            let bcast = |m: &ElemAccess| -> &'static str {
                match (m.varies_along(row_ax.var), m.varies_along(col_ax.var)) {
                    (true, true) => "",             // tile-for-tile
                    (true, false) => "_bcast_cols", // one scalar per ROW, spread along the columns
                    (false, true) => "_bcast_rows", // one value per COLUMN, spread down the rows
                    (false, false) => "_bcast_scalar",
                }
            };
            // ★ THE PRINTER HAS INSTRUCTIONS FOR THE FOUR TILE-ALIGNED BROADCAST FORMS AND NOTHING
            // ELSE. A map with an offset, a stride or an indirect term (a transpose, a window, a
            // gather) would be spelled as a tile-for-tile copy if only `varies_along` were consulted,
            // and a fold over the ROW axis would be printed as a pointwise op — so the exact forms
            // are checked and everything else is refused by name.
            if matches!(row_ax.kind, IterKind::Reduce { .. }) {
                return Err(SpatialLowerError::Unsupported("a Generic reduction over the row axis on Tensix (reduce_tile folds a tile's columns only)"));
            }
            // ★ AND EVERY MAP MUST HAVE THE BUFFER IT READS. The width below is a PAGE width, and a
            // map without an operand buffer behind it has no page — so defaulting to one tile does not
            // describe anything, it just picks the value that makes a 32-wide stride look legal. That
            // is the worst kind of default: one nothing on the real path ever reaches, so the only
            // thing that ever exercises it is this check, which is the very thing meant to catch the
            // mismatch. The operand-page bug this loop caught (a fused row op's `cols_t`-tile page
            // collapsed to one while its maps still stepped a whole row) was found BECAUSE the widths
            // are real; a silent 32 here would have let the same shape through unnamed.
            if maps.len() != ins.len() {
                return Err(SpatialLowerError::Unsupported("a Generic whose access maps and operand buffers do not correspond one to one — a map with no buffer has no page width to be checked against"));
            }
            for (i, m) in maps.iter().enumerate() {
                let width = ctx.p.buffer(ins[i]).kind.page_tiles() * tile_w;
                if m.broadcast_class(row_ax.var, col_ax.var, width).is_none() {
                    return Err(SpatialLowerError::Unsupported("a Generic access map with an offset, a stride or an indirect term the Tensix broadcast family cannot address"));
                }
            }
            if out.indirect.is_some() {
                return Err(SpatialLowerError::Unsupported("a Generic output map with an indirect term on Tensix (a scatter store is B-P5)"));
            }
            if let IterKind::Reduce { pool } = col_ax.kind {
                // A reduction folds the column axis. tt-metal's reduce takes a scaler operand, so the
                // legalizer supplies it as the LAST operand — the one whose map is constant in both
                // axes. That is a Tensix requirement, and it is checked rather than assumed.
                if ins.len() < 2
                    || maps
                        .last()
                        .map(|m| m.varies_along(row_ax.var) || m.varies_along(col_ax.var))
                        .unwrap_or(true)
                {
                    return Err(SpatialLowerError::Unsupported(
                        "a Generic reduction whose last operand is not a constant scaler tile (Tensix `reduce_tile` requires one)",
                    ));
                }
                let (x, sc) = (ctx.cb(ins[0]), ctx.cb(ins[ins.len() - 1]));
                let pool = match pool {
                    ReducePool::Sum => "SUM",
                    ReducePool::Max => "MAX",
                    ReducePool::ArgMax => return Err(SpatialLowerError::Unsupported("an index-valued ArgMax reduction on Tensix (reduce_tile has no argmax pool)")),
                };
                e.line(format!(
                    "reduce_init<PoolType::{pool}, ReduceDim::REDUCE_ROW>({x}, {sc}, {out_cb});"
                ));
                e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
                e.line(format!(
                    "reduce_tile<PoolType::{pool}, ReduceDim::REDUCE_ROW>({x}, {sc}, t, 0, 0);"
                ));
                e.close();
                e.line(format!("reduce_uninit({x});"));
                return Ok(());
            }
            match body {
                // `f(expr)` — whatever produced the destination registers, then one SFPU instruction.
                // Delegating to `emit_body` rather than requiring a bare operand is what lets a FUSED
                // transform lower: softmax's `exp(x - rowmax)` has a broadcast subtract inside the
                // exponential, and that inner half is exactly what `emit_body` knows how to produce.
                TensorBody::Unary { .. } => {
                    emit_body(ctx, e, body, ins, maps, tiles, row_ax, col_ax, &out_cb)
                }
                // `expr ⊕ <literal>` — one SFPU scalar instruction on top of whatever produced DST.
                // This is what lets `mean = Σx · (1/cols)` be a BODY rather than another operator.
                TensorBody::Add(a, b) | TensorBody::Sub(a, b) | TensorBody::Mul(a, b)
                    if matches!(**b, TensorBody::Const(_)) =>
                {
                    let TensorBody::Const(c) = **b else {
                        unreachable!()
                    };
                    emit_body(ctx, e, a, ins, maps, tiles, row_ax, col_ax, &out_cb)?;
                    let stem = match body {
                        TensorBody::Add(..) => "add",
                        TensorBody::Sub(..) => "sub",
                        _ => "mul",
                    };
                    emit_scalar(e, stem, c, tiles)
                }
                // `a ⊕ b` — the broadcast family is chosen by b's access map.
                TensorBody::Add(a, b) | TensorBody::Sub(a, b) | TensorBody::Mul(a, b)
                    if matches!(**a, TensorBody::Operand(_))
                        && matches!(**b, TensorBody::Operand(_)) =>
                {
                    let (ia, ib) = (
                        if let TensorBody::Operand(i) = **a {
                            i
                        } else {
                            0
                        },
                        if let TensorBody::Operand(i) = **b {
                            i
                        } else {
                            0
                        },
                    );
                    let stem = match body {
                        TensorBody::Add(..) => "add",
                        TensorBody::Sub(..) => "sub",
                        _ => "mul",
                    };
                    let (ca, cb_) = (ctx.cb(ins[ia]), ctx.cb(ins[ib]));
                    let axis = bcast(&maps[ib]);
                    e.line(format!("binary_op_init_common({ca}, {cb_}, {out_cb});"));
                    if axis.is_empty() {
                        e.line(format!("{stem}_tiles_init({ca}, {cb_});"));
                        e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
                        e.line(format!("{stem}_tiles({ca}, {cb_}, t, t, t);"));
                    } else {
                        // `mul_bcast_cols_init` / `add_bcast_rows_init` — NOT the deprecated `_short`
                        // spellings, which tt-metal removes after 2026-09-15.
                        e.line(format!("{stem}{axis}_init({ca}, {cb_});"));
                        e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
                        let src = if axis == "_bcast_cols" { "0" } else { "t" };
                        e.line(format!("{stem}_tiles{axis}({ca}, {cb_}, t, {src}, t);"));
                    }
                    e.close();
                    Ok(())
                }
                // ★ A COMPOSED BODY, VIA THE DESTINATION REGISTERS. `f(a) ⊕ b` is not two kernels: the
                // inner result is already sitting in DST, and `mul_reuse_dest_tiles` takes ONE operand
                // from DST instead of from a second circular buffer. So a fused body costs one
                // register window and no round trip, which is exactly what `fuse_generic` is for.
                // Broadcast forms are excluded on purpose — the `_bcast_` family reads two CBs and has
                // no dest-reuse spelling, so a composition involving one is refused rather than
                // silently mis-emitted.
                TensorBody::Add(a, b) | TensorBody::Sub(a, b) | TensorBody::Mul(a, b)
                    if matches!(**b, TensorBody::Operand(_)) =>
                {
                    let j = if let TensorBody::Operand(j) = **b {
                        j
                    } else {
                        0
                    };
                    if maps
                        .get(j)
                        .map(|m| !m.varies_along(row_ax.var) || !m.varies_along(col_ax.var))
                        .unwrap_or(true)
                    {
                        return Err(SpatialLowerError::Unsupported(
                            "a composed Generic body whose second operand is broadcast (no dest-reuse form exists on Tensix)",
                        ));
                    }
                    emit_body(ctx, e, a, ins, maps, tiles, row_ax, col_ax, &out_cb)?;
                    let stem = match body {
                        TensorBody::Add(..) => "add",
                        TensorBody::Sub(..) => "sub",
                        _ => "mul",
                    };
                    let cbj = ctx.cb(ins[j]);
                    e.line(format!(
                        "{stem}_reuse_dest_init<EltwiseBinaryReuseDestType::DEST_TO_SRCA>({cbj});"
                    ));
                    e.open(format!("for (uint32_t t = 0; t < {tiles}u; ++t)"));
                    e.line(format!("{stem}_reuse_dest_tiles<EltwiseBinaryReuseDestType::DEST_TO_SRCA>({cbj}, t, t);"));
                    e.close();
                    Ok(())
                }
                other => Err(SpatialLowerError::Unsupported(match other {
                    TensorBody::Operand(_) => {
                        "a Generic body that is a bare operand copy on Tensix"
                    }
                    TensorBody::Const(_) => "a Generic body that is a bare constant on Tensix",
                    TensorBody::Select { .. } => {
                        "a piecewise Select body on Tensix (no predicated tile sequence is printed)"
                    }
                    _ => "a Generic body shape this printer has no instruction sequence for",
                })),
            }
        }
        ComputeOp::Requant { .. } => Err(SpatialLowerError::Unsupported(
            "an int8 requant epilogue on Tensix (P4b)",
        )),
        ComputeOp::Bias { .. } => Err(SpatialLowerError::Unsupported(
            "a bias epilogue on Tensix (P4)",
        )),
        ComputeOp::Relu => Err(SpatialLowerError::Unsupported(
            "a relu epilogue on Tensix (P4)",
        )),
        _ => Err(SpatialLowerError::Unsupported(
            "a non-matmul compute op on Tensix (P4)",
        )),
    }
}

/// The kernel source of a class.
/// The kernel source of a class and its full runtime-argument layout (site slots, then shape parameters).
/// A short, stable profiler zone name for a kernel class — what the timeline is labelled with.
fn zone_name(class: &KernelClass) -> String {
    let stem = class
        .file_name
        .strip_suffix(".cpp")
        .unwrap_or(&class.file_name);
    format!("LOGOS-{}", stem.to_uppercase())
}

pub fn print_kernel(
    p: &SpatialProgram,
    target: &SpatialTarget,
    cbs: &CbPlan,
    class: &KernelClass,
    slots: &[ArgSlot],
    exact_f32: bool,
) -> Result<(String, Vec<ArgSlot>), SpatialLowerError> {
    let _ = exact_f32;
    let sched = p.schedule.as_ref().unwrap();
    let rep = &sched.programs[class.representative];
    let ops: Vec<ChoreoOp> = rep.ops.iter().map(|t| t.op.clone()).collect();
    let plan = p.work.plan.unwrap();
    let mut written: Vec<BufferId> = Vec::new();
    let mut multicast: Vec<BufferId> = Vec::new();
    walk_ops(&ops, 1, &mut |op, _| match op {
        ChoreoOp::ReadTiles { into, .. } => {
            let t = cbs.slot(*into).template;
            if !written.contains(&t) {
                written.push(t);
            }
        }
        ChoreoOp::McastSend { buf, .. } => {
            let t = cbs.slot(*buf).template;
            if !multicast.contains(&t) {
                multicast.push(t);
            }
            if !written.contains(&t) {
                written.push(t);
            }
        }
        _ => {}
    });
    let compute = rep.engine == Engine::Compute;
    let mut pack_dtypes: Vec<crate::codegen_sva::fabric_target::Precision> = Vec::new();
    walk_ops(&ops, 1, &mut |op, _| {
        if let ChoreoOp::PackAcc { to, .. } = op {
            let d = p.buffer(*to).dtype;
            if !pack_dtypes.contains(&d) {
                pack_dtypes.push(d);
            }
        }
    });
    let mut pack_accumulates = false;
    walk_ops(&ops, 1, &mut |op, _| {
        if let ChoreoOp::PackAcc {
            accumulate: true, ..
        } = op
        {
            pack_accumulates = true;
        }
    });
    let ctx = Ctx {
        p,
        cbs,
        slots,
        mode: item_mode(p, class)?,
        grid_nx: plan.grid.1,
        stride_x: target.sites.x.max(1),
        stride_y: target.sites.y.max(1),
        written,
        multicast,
        params_as_args: !compute,
        pack_formats: pack_dtypes.len(),
        pack_accumulates,
        pack_state: RefCell::new(None),
        params: RefCell::new(Vec::new()),
        site_slots: slots.len(),
    };
    let mut e = Emitter::new();
    e.line(format!("// {} — printed from the spatial IR ({}); every statement is the image of a choreography op.", class.file_name, p.name));
    if compute {
        e.line("#include <cstdint>");
        e.line("#include \"api/compute/tile_move_copy.h\"");
        e.line("#include \"api/compute/matmul.h\"");
        e.line("#include \"api/compute/compute_kernel_hw_startup.h\"");
        e.line("#include \"api/compute/reconfig_data_format.h\"");
        // The device profiler's kernel-side header. A movement kernel gets it through `dataflow_api.h`;
        // a compute kernel has to ask, as the vendor's own profiler examples do.
        e.line("#include \"tools/profiler/kernel_profiler.hpp\"");
        // The compute engine is the one whose work scales with the reduction depth, and it is the one that
        // has never been timed: a TRISC has no NOC of its own, so it cannot write a result back the way a
        // movement kernel can. It can still READ its core's wall clock and print, which is enough to answer
        // whether the counter moves with K at all.
        e.line("#include \"risc_common.h\"");
        e.line("#include \"api/debug/dprint.h\"");
        // ★ THE HEADERS FOLLOW THE OPS, NOT A FIXED LIST. Every engine call this file prints needs its
        // declaration, and a kernel that emitted `add_tiles` without `eltwise_binary.h` would lower
        // cleanly here and fail only at the tt-metal build — the one place the failure is expensive to
        // read. Deriving the set from the choreography means a new arm cannot forget its header.
        for inc in compute_includes(&ops) {
            e.line(format!("#include \"{inc}\""));
        }
    } else {
        e.line("#include <stdint.h>");
        e.line("#include \"api/dataflow/dataflow_api.h\"");
        e.line("#include \"hostdevcommon/common_values.hpp\"");
        if super::args::stamps_cycles(&ops) {
            // `get_timestamp_32b()` reads the core's own wall-clock register. The build already puts the
            // arch's internal include directory on the path, so this resolves without an arch-specific path
            // in the generated source.
            e.line("#include \"risc_common.h\"");
        }
    }
    // Circular buffers touched by this program.
    let mut touched: Vec<BufferId> = Vec::new();
    walk_ops(&ops, 1, &mut |op, _| {
        let bufs: Vec<BufferId> = match op {
            ChoreoOp::Reserve { buf, .. }
            | ChoreoOp::Push { buf, .. }
            | ChoreoOp::Wait { buf, .. }
            | ChoreoOp::Pop { buf, .. }
            | ChoreoOp::McastSend { buf, .. } => vec![*buf],
            ChoreoOp::ReadTiles { into, .. } => vec![*into],
            ChoreoOp::WriteTiles { from, .. } => vec![*from],
            ChoreoOp::Compute {
                ins, acc_reload, ..
            } => ins.iter().copied().chain(*acc_reload).collect(),
            ChoreoOp::PackAcc { to, .. } => vec![*to],
            _ => Vec::new(),
        };
        for b in bufs {
            let t = cbs.slot(b).template;
            if !touched.iter().any(|x| cbs.slot(*x).template == t) {
                touched.push(b);
            }
        }
    });
    let mut cb_slots: Vec<&super::cb::CbSlot> = touched.iter().map(|b| cbs.slot(*b)).collect();
    cb_slots.sort_by_key(|s| s.index);
    for s in &cb_slots {
        e.line(format!("constexpr uint32_t {} = {}u;", s.c_name(), s.index));
        e.line(format!(
            "constexpr uint32_t TILE_BYTES_{} = {}u;",
            s.c_name(),
            s.tile_bytes
        ));
    }
    e.open("void kernel_main()");
    // One device-profiler zone spanning the whole kernel. Without a zone the runtime finds unpaired markers
    // and aborts the run, so this is what makes `TT_METAL_DEVICE_PROFILER=1` usable at all — and per-core
    // timestamps are what a head-to-head would need ON SILICON. ★ On ttsim they are as flat as the global
    // clock: 8x the reduction depth at a fixed plan moves this counter 372 -> 373. ttsim is a functional
    // model and does not simulate per-core timing, so nothing read here measures the program. What does is
    // `spatial::work` — instructions, transactions, bytes, retired matmuls. See ttsim_instrument_audit.json.
    // The macro
    // compiles to `(void(sizeof(name)))` when the profiler is off, so an ordinary run is byte-identical.
    e.line(format!("DeviceZoneScopedN(\"{}\");", zone_name(class)));
    for (i, s) in slots.iter().enumerate() {
        let decl = match s.source {
            ArgSource::SemHandle(_) => format!(
                "const uint32_t {} = get_semaphore(get_arg_val<uint32_t>({i}));",
                s.name.replace("_id", "_addr")
            ),
            _ => format!("const uint32_t {} = get_arg_val<uint32_t>({i});", s.name),
        };
        e.line(decl);
    }
    for s in slots {
        if let ArgSource::SemHandle(sem) = s.source {
            let n = sem.idx();
            e.line(format!("volatile tt_l1_ptr uint32_t* sem{n}_ptr = reinterpret_cast<volatile tt_l1_ptr uint32_t*>(sem{n}_addr);"));
        }
    }
    if compute {
        // The packer is configured for the FINAL output buffer (the store's source), as the vendor does.
        let out = p
            .stages
            .iter()
            .find(|s| s.site == Some(rep.site) && matches!(s.kind, StageKind::Store { .. }))
            .and_then(|s| s.ins.first().copied())
            .ok_or(SpatialLowerError::Unsupported(
                "a compute site without a store",
            ))?;
        let mm_ins = {
            let mut found = None;
            walk_ops(&ops, 1, &mut |op, _| {
                if let ChoreoOp::Compute { ins, .. } = op {
                    if found.is_none() {
                        found = Some(ins.clone());
                    }
                }
            });
            found.ok_or(SpatialLowerError::Unsupported(
                "a compute program without a compute op",
            ))?
        };
        // ★ THE PREAMBLE MUST NOT ASSUME TWO OPERANDS. It configures the two source registers from the
        // first compute op's inputs, which reads naturally for a matmul and for anything binary — and a
        // ONE-INPUT op (a bare row reduction, an SFPU map) then indexed past the end and PANICKED, which
        // is worse than any refusal: the printer never reached `emit_compute`, so the typed
        // `Unsupported` that op already carried was unreachable. Found by
        // `spatial_backend_totality_lock` on its first run.
        let in0 = *mm_ins.first().ok_or(SpatialLowerError::Unsupported(
            "a compute op with no operands",
        ))?;
        let in1 = mm_ins.get(1).copied().unwrap_or(in0);
        e.line(format!(
            "compute_kernel_hw_startup<SrcOrder::Reverse>({}, {}, {});",
            ctx.cb(in0),
            ctx.cb(in1),
            ctx.cb(out)
        ));
        // The prologue configures the engine for the shape the body will hand it.
        let mut init = format!("matmul_init({}, {});", ctx.cb(in0), ctx.cb(in1));
        walk_ops(&ops, 1, &mut |op, _| {
            if let ChoreoOp::Compute {
                op:
                    ComputeOp::MatmulTiles {
                        block,
                        granularity: Granularity::PerSubBlock,
                        ..
                    },
                ..
            } = op
            {
                init = format!(
                    "matmul_block_init({}, {}, 0, {}u, {}u, {}u);",
                    ctx.cb(in0),
                    ctx.cb(in1),
                    block.osw,
                    block.osh,
                    block.ibw
                );
            }
        });
        e.line(init);
    } else {
        // ── A MOVEMENT KERNEL'S TENSOR ACCESSORS ────────────────────────────────────────────────
        //
        // One `TensorAccessor` per tensor this kernel touches, CHAINED: each one's compile-time args
        // begin where the previous one's ended. That chain is the other half of a contract the host
        // sets when it appends `TensorAccessorArgs(*buf_x).append_to(ca_i)` per tensor in id order —
        // get the order or the offsets wrong here and the kernel reads another tensor's descriptor.
        let mut prev: Option<String> = None;
        for s in slots {
            let ArgSource::TensorAddr(_) = s.source else {
                continue;
            };
            let base = s.name.strip_suffix("_addr").unwrap_or(&s.name).to_string();
            let offset = match &prev {
                None => "0".to_string(),
                Some(p) => format!("{p}_args.next_compile_time_args_offset()"),
            };
            e.line(format!(
                "constexpr auto {base}_args = TensorAccessorArgs<{offset}>();"
            ));
            e.line(format!(
                "const auto acc_{base} = TensorAccessor({base}_args, {});",
                s.name
            ));
            prev = Some(base);
        }
        if super::args::stamps_cycles(&ops) {
            let offset = match &prev {
                None => "0".to_string(),
                Some(p) => format!("{p}_args.next_compile_time_args_offset()"),
            };
            e.line(format!(
                "constexpr auto cyc_args = TensorAccessorArgs<{offset}>();"
            ));
            e.line("const auto acc_cyc = TensorAccessor(cyc_args, cycles_addr);");
        }
        for s in &cb_slots {
            if ctx.multicast.contains(&s.template) {
                e.line(format!(
                    "uint32_t wp_{0} = 0, wp_{0}_start = 0;",
                    s.c_name()
                ));
            } else if ctx.written.contains(&s.template) {
                e.line(format!("uint32_t wp_{0} = 0;", s.c_name()));
            }
        }
        // A sender's local VALID copy is the source of its flag multicast.
        let mut valid_sems: Vec<SemId> = Vec::new();
        walk_ops(&ops, 1, &mut |op, _| {
            if let ChoreoOp::McastSetValid { sem, .. } = op {
                if !valid_sems.contains(sem) {
                    valid_sems.push(*sem);
                }
            }
        });
        for s in valid_sems {
            e.line(format!("*(sem{}_ptr) = VALID;", s.idx()));
        }
    }
    let has_item_loop = matches!(ops.first(), Some(ChoreoOp::Loop { var, .. }) if *var == LV_ITEM);
    let mut body = Emitter::new();
    body.indent = 1;
    let (item, coords) = uses_coords(&ctx, &ops, &[]);
    if item && !has_item_loop {
        ctx.coord_lines(&mut body, false, coords);
    }
    let mut scope = Vec::new();
    emit_ops(&ctx, &mut body, &ops, compute, &mut scope)?;
    // Shape parameters discovered while printing the body: declared after the site slots, in first-use order.
    for (j, s) in ctx.params.borrow().iter().enumerate() {
        e.line(format!(
            "const uint32_t {} = get_arg_val<uint32_t>({});",
            s.name,
            ctx.site_slots + j
        ));
    }
    if !compute {
        let body_text = body.lines.join("\n");
        for s in &cb_slots {
            if body_text.contains(&format!("tile_bytes_{}", s.c_name())) {
                e.line(format!(
                    "const uint32_t tile_bytes_{0} = get_tile_size({0});",
                    s.c_name()
                ));
            }
        }
    }
    // One read at each end, outside every loop, so timing the program cannot change it.
    if compute || super::args::stamps_cycles(&ops) {
        e.line("const uint32_t t_cyc0 = get_timestamp_32b();");
    }
    e.lines.extend(body.lines);
    if compute {
        e.line("DPRINT(\"LOGOS_COMPUTE_CYCLES {}\\n\", (uint32_t)(get_timestamp_32b() - t_cyc0));");
    } else if super::args::stamps_cycles(&ops) {
        e.line("noc_inline_dw_write(get_noc_addr(cycles_slot, acc_cyc), get_timestamp_32b() - t_cyc0);");
        e.line("noc_async_write_barrier();");
    }
    e.close();
    let mut all = slots.to_vec();
    all.extend(ctx.params.borrow().iter().cloned());
    Ok((e.text(), all))
}
