//! The exact extension on RISC-V. A register holding a value of the extension (an infinitesimal, a hyperreal, an
//! algebraic number, an ordinal, a residue-dependent or provisional answer, and every list of them) holds its
//! `LTower` cell, a collected cell around a handle into the linked value runtime, so the program computes with the
//! compiler's own value model, as the C backends and linked WebAssembly do.
//!
//! Which registers hold one is settled per exact web ([`Emitter::infer_kinds`]). An operation over a cell is one
//! call into the shim's adapter (`tower.c`), each operand passed with the shape of its slot, so the adapter makes the
//! runtime's value of whatever the slot holds and the assembly keeps nothing across the call. An op the tier's own
//! lowering takes reads its operands in the kinds it knows: a cell read as a machine integer (an index, a bound) is
//! presented as the `Int` it is, and a value stored where a cell is held (a parameter, a result, a global, a field)
//! is presented as the extension's value, each in a scratch slot of the frame. A definition of an exact web that
//! computes another kind lowers as what it computes and is made the extension's value after it.

use super::{kind_at, Composite, Emit, Emitter, Kind, RiscVLowerError, SCRATCH_SLOTS};
use crate::semantics::builtins::BuiltinId;
use crate::vm::tower::{binary, by_constant, lifts};
use crate::vm::variables::defined;
use crate::vm::{Constant, Op};

impl Emitter<'_> {
    /// Lower `op` in a program the exact extension reaches: through the adapter where it computes with the extension,
    /// else through the tier's own lowering over its operands as presented, then making a definition of an exact web
    /// that computed another kind the extension's value.
    pub(super) fn lower_over_tower(&mut self, pc: usize, op: &Op, kinds: &[Kind], frame: i64) -> Emit {
        if self.lower_tower(op, kinds)? {
            let mut written = Vec::new();
            self.for_each_written(op, |w| written.push(w));
            for w in written {
                if self.may_be_nothing.contains(&w) {
                    self.store_flag(w, "zero");
                }
            }
            return Ok(());
        }
        let seen = self.present_operands(op, kinds)?;
        let seen = self.present_numbers(op, &seen)?;
        let lowered = self.lower_op_over_nothing(pc, op, &seen, frame);
        self.presented.clear();
        lowered?;
        let natural = self.tower_from.borrow().get(&pc).copied();
        if let (Some(k), Some(w)) = (natural, defined(op)) {
            let nothing = matches!(*op, Op::LoadConst { idx, .. }
                if matches!(self.prog.constants[idx as usize], Constant::Nothing));
            if nothing {
                let shape = self.nothing_shape();
                self.line(&format!("la a1, {shape}"));
            } else {
                self.operand_shape(w, k, "a1")?;
            }
            self.load_slot(w, "a0");
            self.line("call logos_tower_of");
            self.store_slot(w, "a0");
            self.fx.keeps.retain(|r| *r != w);
            self.fresh(&[w]);
        }
        Ok(())
    }

    /// Whether the adapter makes the extension's value of a value of kind `k`: a scalar it has one for, a cell, and a
    /// list or tuple of those.
    fn converts(&self, k: Kind) -> bool {
        match k {
            Kind::Int
            | Kind::Bool
            | Kind::Float
            | Kind::Text
            | Kind::Rational
            | Kind::Date
            | Kind::Moment
            | Kind::Span
            | Kind::CrdtCounter
            | Kind::Tower
            | Kind::SeqInt
            | Kind::SeqFloat
            | Kind::SeqText
            | Kind::SeqBool
            | Kind::SeqUnknown => true,
            Kind::Composite(_) => match self.as_composite(k) {
                Some(Composite::Seq(e)) => self.converts(e),
                Some(Composite::Tuple(ps)) => ps.iter().all(|p| self.converts(*p)),
                _ => false,
            },
            _ => false,
        }
    }

    /// The shape the adapter reads a slot of kind `k` by when it makes the extension's value of it
    /// (`logos_tower_cell`): its own, or for a list nothing was put into, an empty list's.
    pub(super) fn tower_shape(&mut self, k: Kind) -> Result<String, RiscVLowerError> {
        if !self.converts(k) {
            return Err(RiscVLowerError::Unsupported(
                "a value of the exact extension made from a value this tier does not convert",
            ));
        }
        self.shape(if k == Kind::SeqUnknown { Kind::SeqInt } else { k })
    }

    /// The label of the shape of `nothing` (form 23), which the adapter makes the runtime's `nothing` of.
    fn nothing_shape(&mut self) -> String {
        if let Some(label) = &self.nothing_shape {
            return label.clone();
        }
        let label = ".Lshape_nothing".to_string();
        self.ro
            .table(label.clone(), vec!["23".to_string(), "0".to_string(), format!("{label}_parts")]);
        self.ro.table(format!("{label}_parts"), Vec::new());
        self.nothing_shape = Some(label.clone());
        label
    }

    /// The label of the table, one word per function of the program, of the shape the adapter reads a function's
    /// answer by when a real number is defined by it: a closure body of one `Int` answering a number, or `0`.
    fn defined_answers(&mut self) -> Result<String, RiscVLowerError> {
        if let Some(label) = &self.defined_answers {
            return Ok(label.clone());
        }
        let mut shapes = Vec::with_capacity(self.prog.functions.len());
        for fi in 0..self.prog.functions.len() {
            let takes_an_int = self.prog.functions[fi].param_count == 1
                && matches!(self.param_kinds.borrow()[fi].first(), Some(Kind::Int));
            let answers = self.returns.borrow()[fi];
            let defines = self.closure_bodies[fi]
                && takes_an_int
                && matches!(answers, Kind::Int | Kind::Rational | Kind::Tower);
            shapes.push(if defines { self.tower_shape(answers)? } else { "0".to_string() });
        }
        let label = ".Ldefined_answers".to_string();
        self.ro.table(label.clone(), shapes);
        self.defined_answers = Some(label.clone());
        Ok(label)
    }

    /// Load into `into` the shape the adapter reads register `reg`, of kind `k`, by: `nothing`'s where the register
    /// holds the `nothing` literal or its flag says it holds `nothing`, else its kind's.
    fn operand_shape(&mut self, reg: u16, k: Kind, into: &str) -> Emit {
        if self.is_nothing_literal(self.pc, reg) {
            let nothing = self.nothing_shape();
            self.line(&format!("la {into}, {nothing}"));
            return Ok(());
        }
        let label = self.tower_shape(k)?;
        self.line(&format!("la {into}, {label}"));
        if self.may_be_nothing.contains(&reg) {
            let nothing = self.nothing_shape();
            self.load_flag(reg, "t0");
            self.line("beqz t0, 1f");
            self.line(&format!("la {into}, {nothing}"));
            self.line("1:");
        }
        Ok(())
    }

    /// Load register `reg` into `value` and the shape it is read by into `shape`: one operand of an adapter call.
    fn tower_operand(&mut self, reg: u16, kinds: &[Kind], value: &str, shape: &str) -> Emit {
        self.operand_shape(reg, self.kind_of(kinds, reg), shape)?;
        self.load_slot(reg, value);
        Ok(())
    }

    /// The label of the table of shapes of the `count` registers from `start`, for an adapter call that reads a run of
    /// consecutive registers (`logos_tower_list`, `logos_tower_call`).
    fn shape_run(&mut self, start: u16, count: u16, kinds: &[Kind]) -> Result<String, RiscVLowerError> {
        let mut shapes = Vec::with_capacity(count as usize);
        for reg in start..start + count {
            if self.is_nothing_literal(self.pc, reg) {
                shapes.push(self.nothing_shape());
            } else if self.may_be_nothing.contains(&reg) {
                return Err(RiscVLowerError::Unsupported(
                    "a value that may be nothing among the values a run of the exact extension reads",
                ));
            } else {
                shapes.push(self.tower_shape(self.kind_of(kinds, reg))?);
            }
        }
        let label = format!(".Ltower_run_{}", self.pc);
        self.ro.table(label.clone(), shapes);
        Ok(label)
    }

    /// Lower `op` where it computes with the extension: a builtin of the extension, a list holding a value of it, a
    /// lifting builtin, an operator, an item, a change to a list or a field, a length, a membership, a `Show`, a
    /// condition or a loop over a cell. `false` for an op that reads no cell, or only moves one, which the tier's own
    /// lowering takes.
    fn lower_tower(&mut self, op: &Op, kinds: &[Kind]) -> Result<bool, RiscVLowerError> {
        let t = |r: u16| kind_at(kinds, r) == Kind::Tower;
        let text = |r: u16| kind_at(kinds, r) == Kind::Text;
        // A value that crosses to a task or through a channel: each cell is checked as it crosses (a real
        // number a function of the program defines stays with the program), then the tier's own lowering
        // passes it.
        let crossing: Vec<u16> = match *op {
            Op::Spawn { args_start, arg_count, .. } | Op::SpawnHandle { args_start, arg_count, .. } => {
                (args_start..args_start + arg_count).filter(|r| t(*r)).collect()
            }
            Op::ChanSend { val, .. } | Op::ChanTrySend { val, .. } if t(val) => vec![val],
            _ => Vec::new(),
        };
        if !crossing.is_empty() {
            for reg in crossing {
                self.load_slot(reg, "a0");
                self.line("call logos_tower_crossing");
            }
            return Ok(false);
        }
        match *op {
            Op::CallBuiltin {
                dst,
                builtin: BuiltinId::DefinedReal,
                args_start,
                ..
            } => {
                if self.kind_of(kinds, args_start + 1) != Kind::Int {
                    return Err(RiscVLowerError::Unsupported("a real number's base held as no Int"));
                }
                let answers = self.defined_answers()?;
                for (i, reg) in (args_start..args_start + 4).enumerate() {
                    self.load_slot(reg, &format!("a{i}"));
                }
                self.line(&format!("la a4, {answers}"));
                self.line("call logos_tower_define");
                self.store_slot(dst, "a0");
                self.inspects(&(args_start..args_start + 4).collect::<Vec<_>>());
                self.fresh(&[dst]);
            }
            Op::CallBuiltin {
                dst,
                builtin,
                args_start,
                arg_count,
            } if builtin.extension().is_some() => {
                let name = builtin.extension().expect("a builtin of the extension").name();
                let shapes = self.shape_run(args_start, arg_count, kinds)?;
                let label = self.ro.intern_text(name);
                self.line(&format!("la a0, {label}"));
                self.line(&format!("li a1, {arg_count}"));
                self.slot_addr(args_start, "a2");
                self.line(&format!("la a3, {shapes}"));
                self.line("call logos_tower_call");
                self.store_slot(dst, "a0");
                self.inspects(&(args_start..args_start + arg_count).collect::<Vec<_>>());
                self.fresh(&[dst]);
            }
            Op::NewList { dst, start, count } if (start..start + count).any(t) => {
                let shapes = self.shape_run(start, count, kinds)?;
                self.line(&format!("li a0, {count}"));
                self.slot_addr(start, "a1");
                self.line(&format!("la a2, {shapes}"));
                self.line("call logos_tower_list");
                self.store_slot(dst, "a0");
                self.inspects(&(start..start + count).collect::<Vec<_>>());
                self.fresh(&[dst]);
            }
            Op::CallBuiltin {
                dst,
                builtin,
                args_start,
                arg_count,
            } if lifts(builtin).is_some() && (args_start..args_start + arg_count).any(t) => {
                let f = lifts(builtin).expect("a lifting builtin");
                if arg_count > 2 {
                    return Err(RiscVLowerError::Unsupported(
                        "a lifting builtin of more than two values of the exact extension",
                    ));
                }
                for i in 0..arg_count {
                    let (value, shape) = (format!("a{}", 2 * i), format!("a{}", 2 * i + 1));
                    self.tower_operand(args_start + i, kinds, &value, &shape)?;
                }
                self.line(&format!("call logos_tower_{f}"));
                self.store_slot(dst, "a0");
                self.inspects(&(args_start..args_start + arg_count).collect::<Vec<_>>());
                self.fresh(&[dst]);
            }
            Op::Add { lhs, rhs, .. } | Op::AddAssign { dst: lhs, src: rhs } if text(lhs) || text(rhs) => {
                return Ok(false)
            }
            Op::AddAssign { dst, src } if t(dst) || t(src) => {
                self.tower_operand(dst, kinds, "a0", "a1")?;
                self.tower_operand(src, kinds, "a2", "a3")?;
                self.line("call logos_tower_add");
                self.store_slot(dst, "a0");
                self.inspects(&[src]);
                self.fresh(&[dst]);
            }
            Op::Not { dst, src } if t(src) => {
                self.tower_operand(src, kinds, "a0", "a1")?;
                self.line("call logos_tower_not");
                self.store_slot(dst, "a0");
                self.inspects(&[src]);
                self.fresh(&[dst]);
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
            } if t(collection) => {
                self.tower_operand(collection, kinds, "a0", "a1")?;
                self.tower_operand(index, kinds, "a2", "a3")?;
                self.line("call logos_tower_item");
                self.store_slot(dst, "a0");
                self.inspects(&[collection, index]);
                self.fresh(&[dst]);
            }
            Op::ListPush { list, value } if t(list) => {
                self.own_target(list, &[value]);
                self.tower_operand(value, kinds, "a2", "a3")?;
                self.line("call logos_tower_push_own");
                self.inspects(&[value]);
            }
            Op::ListPop { list, dst } if t(list) => {
                self.own_target(list, &[]);
                self.line("call logos_tower_pop_own");
                self.store_slot(dst, "a0");
                self.fresh(&[dst]);
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
            } if t(collection) => {
                self.own_target(collection, &[index, value]);
                self.tower_operand(index, kinds, "a2", "a3")?;
                self.tower_operand(value, kinds, "a4", "a5")?;
                self.line("call logos_tower_set_item_own");
                self.inspects(&[index, value]);
            }
            Op::SliceOp {
                dst,
                collection,
                start,
                end,
            } if t(collection) => {
                self.tower_operand(collection, kinds, "a0", "a1")?;
                self.tower_operand(start, kinds, "a2", "a3")?;
                self.tower_operand(end, kinds, "a4", "a5")?;
                self.line("call logos_tower_slice");
                self.store_slot(dst, "a0");
                self.inspects(&[collection, start, end]);
                self.fresh(&[dst]);
            }
            Op::Length { dst, collection } if t(collection) => {
                self.load_slot(collection, "a0");
                self.line("call logos_tower_length");
                self.store_slot(dst, "a0");
                self.inspects(&[collection]);
            }
            Op::Contains {
                dst,
                collection,
                value,
            } if t(collection) => {
                self.load_slot(collection, "a0");
                self.tower_operand(value, kinds, "a1", "a2")?;
                self.line("call logos_tower_contains");
                self.store_slot(dst, "a0");
                self.inspects(&[collection, value]);
            }
            Op::Show { src } if t(src) => {
                self.load_slot(src, "a0");
                self.line("call logos_tower_show");
                self.inspects(&[src]);
            }
            Op::JumpIfFalse { cond, target } if t(cond) => {
                self.load_slot(cond, "a0");
                self.line("call logos_tower_truth");
                self.line(&format!("beqz a0, .Lpc_{target}"));
                self.inspects(&[cond]);
            }
            Op::JumpIfTrue { cond, target } if t(cond) => {
                self.load_slot(cond, "a0");
                self.line("call logos_tower_truth");
                self.line(&format!("bnez a0, .Lpc_{target}"));
                self.inspects(&[cond]);
            }
            Op::IterPrepare { iterable } if t(iterable) => {
                self.load_slot(iterable, "a0");
                self.line("call logos_tower_items");
                self.line("call logos_iter_begin");
                self.inspects(&[iterable]);
            }
            Op::ListPushField { obj, field, src: value } => {
                let Some((cell, Kind::Tower)) = self.resolve_field(self.kind_of(kinds, obj), field) else {
                    return Ok(false);
                };
                self.field_target(obj, cell, &[value]);
                self.tower_operand(value, kinds, "a4", "a5")?;
                self.line("call logos_tower_field_push_own");
                self.inspects(&[value]);
            }
            ref other => match (binary(other), by_constant(other)) {
                (Some((dst, lhs, rhs, f)), _) if t(lhs) || t(rhs) => {
                    self.tower_operand(lhs, kinds, "a0", "a1")?;
                    self.tower_operand(rhs, kinds, "a2", "a3")?;
                    self.line(&format!("call logos_tower_{f}"));
                    self.store_slot(dst, "a0");
                    self.inspects(&[lhs, rhs]);
                    self.fresh(&[dst]);
                }
                (_, Some((dst, lhs, divisor, f))) if t(lhs) => {
                    self.tower_operand(lhs, kinds, "a0", "a1")?;
                    let int = self.tower_shape(Kind::Int)?;
                    self.line(&format!("li a2, {divisor}"));
                    self.line(&format!("la a3, {int}"));
                    self.line(&format!("call logos_tower_{f}"));
                    self.store_slot(dst, "a0");
                    self.inspects(&[lhs]);
                    self.fresh(&[dst]);
                }
                _ => return Ok(false),
            },
        }
        Ok(true)
    }

    /// Present the operands of `op`, which the tier's own lowering takes, in the kinds it reads them as: a cell read as
    /// a machine integer (the index of a list of the tier's own, a range's bound) as the `Int` it is, and a value of
    /// another kind stored where a cell is held (an argument of a parameter that holds one, a result of a function
    /// that answers one, a global or a field that holds one) as the extension's value. Answers the kinds the lowering
    /// reads.
    fn present_operands(&mut self, op: &Op, kinds: &[Kind]) -> Result<Vec<Kind>, RiscVLowerError> {
        let mut seen = kinds.to_vec();
        use crate::codegen_c::words::IntegerPlace;
        let mut narrow: Vec<(u16, IntegerPlace)> = Vec::new();
        let mut widen: Vec<u16> = Vec::new();
        let t = |r: u16| kind_at(kinds, r) == Kind::Tower;
        let machine_list = |e: &Self, r: u16| !t(r) && !e.keyed(kind_at(kinds, r));
        let read_place = |r: u16| if kind_at(kinds, r) == Kind::Text { IntegerPlace::Text } else { IntegerPlace::ListRead };
        match *op {
            Op::Index {
                collection, index, ..
            }
            | Op::IndexUnchecked {
                collection, index, ..
            } if machine_list(self, collection) && t(index) => narrow.push((index, read_place(collection))),
            Op::SetIndex {
                collection, index, ..
            }
            | Op::SetIndexUnchecked {
                collection, index, ..
            } if machine_list(self, collection) && t(index) => narrow.push((index, IntegerPlace::ListStore)),
            Op::SliceOp {
                collection, start, end, ..
            } if machine_list(self, collection) => {
                narrow.extend([start, end].into_iter().filter(|r| t(*r)).map(|r| (r, IntegerPlace::Slice)))
            }
            Op::SetIndex {
                collection, value, ..
            }
            | Op::SetIndexUnchecked {
                collection, value, ..
            } if matches!(self.as_composite(kind_at(kinds, collection)), Some(Composite::Map(_, Kind::Tower))) => {
                widen.push(value)
            }
            Op::NewRange { start, end, .. } => {
                narrow.extend([start, end].into_iter().filter(|r| t(*r)).map(|r| (r, IntegerPlace::Range)))
            }
            Op::Call {
                func,
                args_start,
                arg_count,
                ..
            } => {
                let params = self.param_kinds.borrow().get(func as usize).cloned().unwrap_or_default();
                widen.extend(
                    (0..arg_count)
                        .filter(|i| params.get(*i as usize) == Some(&Kind::Tower))
                        .map(|i| args_start + i),
                );
            }
            Op::CallValue {
                callee,
                args_start,
                arg_count,
                ..
            } => {
                if let Some(Composite::Closure(f)) = self.as_composite(kind_at(kinds, callee)) {
                    let params = self.param_kinds.borrow().get(f as usize).cloned().unwrap_or_default();
                    widen.extend(
                        (0..arg_count)
                            .filter(|i| params.get(*i as usize) == Some(&Kind::Tower))
                            .map(|i| args_start + i),
                    );
                }
            }
            Op::Return { src } => {
                let answers = self.current_fn.map(|fi| self.returns.borrow()[fi]);
                if answers == Some(Kind::Tower) {
                    widen.push(src);
                }
            }
            Op::GlobalSet { idx, src } => {
                if self.global_kinds.get(idx as usize) == Some(&Kind::Tower) {
                    widen.push(src);
                }
            }
            Op::StructInsert { obj, field, value } => {
                if let Some((_, Kind::Tower)) = self.resolve_field(kind_at(kinds, obj), field) {
                    widen.push(value);
                }
            }
            _ => {}
        }
        widen.retain(|r| !t(*r));
        if narrow.len() + widen.len() > SCRATCH_SLOTS {
            return Err(RiscVLowerError::Unsupported(
                "an op with more operands to present than a frame has scratch slots",
            ));
        }
        let mut at = self.scratch_base;
        for (reg, place) in narrow {
            let (past_word, fraction) = self.integer_place(place);
            self.load_slot(reg, "a0");
            self.line(&format!("la a1, {past_word}"));
            self.line(&format!("la a2, {fraction}"));
            self.line("call logos_tower_int_at");
            self.store_at_sp(at, "a0");
            self.presented.insert(reg, at);
            if let Some(k) = seen.get_mut(reg as usize) {
                *k = Kind::Int;
            }
            at += 8;
        }
        for reg in widen {
            self.tower_operand(reg, kinds, "a0", "a1")?;
            self.line("call logos_tower_of");
            self.store_at_sp(at, "a0");
            self.presented.insert(reg, at);
            if let Some(k) = seen.get_mut(reg as usize) {
                *k = Kind::Tower;
            }
            at += 8;
        }
        Ok(seen)
    }
}
