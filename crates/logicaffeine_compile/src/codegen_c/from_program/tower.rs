//! The exact extension on the bytecode walker. A register holding a value of the extension (an infinitesimal, a
//! hyperreal, an algebraic number, an ordinal, a residue-dependent or provisional answer, and every list of them)
//! rides its slot as an `LTower`, a collected cell around a handle into the linked value runtime (`tower.c`), so
//! the walker computes with the compiler's own value model, as the AST generator does.
//!
//! A register is reused for unrelated values, so what holds a cell is decided per *web* ([`webs`]): the definitions
//! that reach one read are one variable, and a web is exact when any of its definitions makes an exact value (or a
//! parameter of it arrives as one). Every definition of an exact web writes a cell, a value of another kind converted on the
//! way in, so every read of the web finds one. An operation over a cell computes through the runtime; one the
//! walker cannot lower over a cell is refused by name.

use super::{kind_at, Emitter, Kind, WalkerRegion};
use crate::semantics::builtins::BuiltinId;
use crate::vm::tower::{binary, by_constant, exact_write, lifts, webs, Webs};
use crate::vm::Op;

/// The cell register `reg` holds.
fn cell(reg: u16) -> String {
    format!("((LTower)(intptr_t)r[{reg}])")
}

impl<'a> Emitter<'a> {
    /// The register an op writes with a value of the extension, by the rule every backend reads ([`exact_write`]),
    /// over the kinds of what it reads.
    pub(super) fn tower_write(&self, op: &Op, cur: &[Kind]) -> Option<(u16, Kind)> {
        exact_write(
            op,
            |r| kind_at(cur, r) == Kind::Tower,
            |r| kind_at(cur, r) == Kind::Text,
            |r| {
                matches!(
                    kind_at(cur, r),
                    Kind::Map(_) | Kind::MapUnknown | Kind::Struct(_) | Kind::StructSynth(_)
                )
            },
        )
        .map(|dst| (dst, Kind::Tower))
    }

    /// `Push src to obj's field`, the field a list of the extension: the record becomes the register's own, the
    /// field's cell the record's own (a cell the record may share with another holder is copied first), and the item
    /// is pushed onto it.
    pub(super) fn push_to_exact_field(
        &mut self,
        obj: u16,
        si: u16,
        slot: usize,
        src: u16,
        kinds: &[Kind],
    ) -> Result<(), String> {
        self.own_target(obj, Kind::Struct(si))?;
        let field = format!("((int64_t*)(intptr_t)r[{obj}])[{slot}]");
        let word = self
            .field_owner_word(obj, Kind::Struct(si))
            .ok_or("an exact field of a record with no owner word")?;
        let own = format!("{field} = (int64_t)(intptr_t)ltower_own((LTower)(intptr_t){field});");
        if slot < 64 {
            self.line(&format!("if (!(({word} >> {slot}) & 1)) {{ {own} {word} |= 1ULL << {slot}; }}"));
        } else {
            self.line(&own);
        }
        let v = self.tower_reg(src, kinds)?;
        self.line(&format!("ltower_push((LTower)(intptr_t){field}, {v});"));
        Ok(())
    }

    /// The exact webs of region `[start, end)` under its kinds `per_pc` and entry kinds `seed` ([`webs`]).
    pub(super) fn tower_webs(
        &self,
        start: usize,
        end: usize,
        holes: &[(usize, usize)],
        per_pc: &[Vec<Kind>],
        seed: &[Kind],
    ) -> Webs {
        let region = WalkerRegion {
            emitter: self,
            start,
            end,
            holes,
        };
        webs(
            &self.prog.code,
            &region,
            seed.len() as u16,
            |reg| seed.get(reg as usize) == Some(&Kind::Tower),
            |pc, w| kind_at(&per_pc[pc - start + 1], w) == Kind::Tower,
        )
    }

    /// The `LTower` of `v`, a slot value of kind `k`: the cell itself, or the runtime's value of what the slot
    /// holds. A collection converts item by item.
    pub(super) fn tower_of(&mut self, v: &str, k: Kind) -> Result<String, String> {
        Ok(match k {
            Kind::Tower => format!("((LTower)(intptr_t)({v}))"),
            Kind::Int => format!("ltower_int({v})"),
            Kind::Bool => format!("ltower_bool((int)({v}))"),
            Kind::Float => format!("ltower_float(as_f64({v}))"),
            Kind::Text => format!("ltower_text((const char*)(intptr_t)({v}))"),
            Kind::Rational => format!("ltower_of_ldec(*(LDec*)(intptr_t)({v}))"),
            Kind::BigInt => format!("ltower_of_lint(lc_xget({v}))"),
            Kind::Nothing => "ltower_nothing()".to_string(),
            Kind::Date => format!("ltower_date({v})"),
            Kind::Moment => format!("ltower_moment({v})"),
            Kind::Time => format!("ltower_time({v})"),
            Kind::Span => format!("ltower_span((int32_t)((int64_t)({v}) >> 32), (int32_t)(int64_t)({v}))"),
            Kind::SeqUnknown | Kind::Never => "ltower_list(0, NULL)".to_string(),
            Kind::SeqInt => format!("ltower_of_seq_i64((Seq_i64*)(intptr_t)({v}))"),
            Kind::SeqFloat => format!("ltower_of_seq_f64((Seq_f64*)(intptr_t)({v}))"),
            Kind::SeqBool => format!("ltower_of_seq_bool((Seq_bool*)(intptr_t)({v}))"),
            Kind::SeqText => format!("ltower_of_seq_str((Seq_str*)(intptr_t)({v}))"),
            Kind::SeqOf(_) => {
                let n = self.fresh();
                let item = self.tower_of(&format!("_ts{n}->data[_ti{n}]"), self.elem_kind(k))?;
                format!(
                    "({{ Seq_i64 *_ts{n} = (Seq_i64*)(intptr_t)({v}); LTower *_tv{n} = (LTower*)lc_gc_alloc((_ts{n}->len ? _ts{n}->len : 1) * sizeof(LTower)); \
                     for (size_t _ti{n} = 0; _ti{n} < _ts{n}->len; _ti{n}++) _tv{n}[_ti{n}] = {item}; ltower_list(_ts{n}->len, _tv{n}); }})"
                )
            }
            Kind::Tuple(ti) => {
                let n = self.fresh();
                let positions = self.tuple_positions(ti);
                let mut items = Vec::with_capacity(positions.len());
                for (i, pk) in positions.iter().enumerate() {
                    items.push(self.tower_of(&format!("_tp{n}[{i}]"), *pk)?);
                }
                if items.is_empty() {
                    "ltower_tuple(0, NULL)".to_string()
                } else {
                    format!(
                        "({{ int64_t *_tp{n} = (int64_t*)(intptr_t)({v}); ltower_tuple({}, (LTower[]){{{}}}); }})",
                        items.len(),
                        items.join(", ")
                    )
                }
            }
            other => return Err(format!("a {other:?} value where an exact value is held")),
        })
    }

    /// The reader the value runtime calls for a function the program defines a real number by, emitted when the
    /// program defines one: `lc_defines(code, env, n)` calls the closure `env` is, by its body, with `n`, and hands
    /// the body's answer back as the runtime's value. A closure body's parameter keeps the kind it declares — only a
    /// direct call refines one — so the phrase's `(n: Int)` takes a machine word; a body that takes another shape, or
    /// answers a value no number is made of, is no function a real number is defined by.
    pub(super) fn emit_defining_reader(&mut self) -> Result<(), String> {
        let defines = self
            .prog
            .code
            .iter()
            .any(|op| matches!(op, Op::CallBuiltin { builtin: BuiltinId::DefinedReal, .. }));
        if !defines {
            return Ok(());
        }
        let bodies: Vec<usize> = self.closure_bodies.iter().copied().collect();
        let mut arms = String::new();
        for b in bodies {
            if self.prog.functions[b].param_count != 1 {
                continue;
            }
            if self.param_kinds[b].first() != Some(&Kind::Int) {
                continue;
            }
            let answers = self.fn_ret.get(b).copied().unwrap_or(Kind::Unknown);
            if !matches!(answers, Kind::Int | Kind::BigInt | Kind::Rational | Kind::Tower) {
                continue;
            }
            let answer = self.tower_of("v", answers)?;
            arms.push_str(&format!(
                "        case {b}: {{ int64_t v = fn_{b}(n, clo, NULL); return ltower_hand_out({answer}); }}\n"
            ));
        }
        self.out.push_str(&format!(
            "static LogosRtValue *lc_defines(void *code, void *env, int64_t n) {{\n    (void)code;\n    int64_t clo = \
             (int64_t)(intptr_t)env;\n    switch ((int)((int64_t*)env)[0]) {{\n{arms}    }}\n    logos_fatal(\"a real \
             number is defined by a function of one Int that answers a number\");\n}}\n\n"
        ));
        Ok(())
    }

    /// The `LTower` register `reg` holds, converted from its kind in `kinds`.
    fn tower_reg(&mut self, reg: u16, kinds: &[Kind]) -> Result<String, String> {
        self.tower_of(&format!("r[{reg}]"), kind_at(kinds, reg))
    }

    /// `r[dst]` set to the cell `value`.
    fn set_tower(&mut self, dst: u16, value: &str) {
        self.line(&format!("r[{dst}] = (int64_t)(intptr_t)({value});"));
    }

    /// Lower `op` where it computes with the extension: `Some(true)` when it did and what it writes is a cell (or it
    /// changed a list in place), `Some(false)` when it did and wrote a machine value (a length, a membership),
    /// `None` for an op that reads no cell, or only moves one, which the walker's own lowering takes.
    pub(super) fn lower_tower(&mut self, op: &Op, kinds: &[Kind], pc: usize) -> Result<Option<bool>, String> {
        let t = |r: u16| kind_at(kinds, r) == Kind::Tower;
        let mut reads_cell = false;
        op.for_each_read_reg(
            |f| self.prog.functions[f as usize].captures.len(),
            |r| reads_cell |= t(r),
        );
        if let Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DefinedReal,
            args_start,
            ..
        } = *op
        {
            let text = |r: u16| format!("(const char*)(intptr_t)r[{r}]");
            let base = match kind_at(kinds, args_start + 1) {
                Kind::Int => format!("r[{}]", args_start + 1),
                Kind::BigInt => format!("lc_xnarrow(r[{}])", args_start + 1),
                other => return Err(format!("a real number's base held as {other:?}")),
            };
            self.set_tower(
                dst,
                &format!(
                    "ltower_define({}, {base}, {}, lc_defines, NULL, (void*)(intptr_t)r[{}])",
                    text(args_start),
                    text(args_start + 2),
                    args_start + 3
                ),
            );
            return Ok(Some(true));
        }
        if let Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            arg_count,
        } = *op
        {
            if let Some(b) = builtin.extension() {
                let mut args = Vec::with_capacity(arg_count as usize);
                for i in 0..arg_count {
                    args.push(self.tower_reg(args_start + i, kinds)?);
                }
                let run = if args.is_empty() {
                    "0, NULL".to_string()
                } else {
                    format!("{}, (LTower[]){{{}}}", args.len(), args.join(", "))
                };
                self.set_tower(dst, &format!("ltower_call(\"{}\", {run})", b.name()));
                return Ok(Some(true));
            }
        }
        // A list written in place whose variable holds a cell is built as the runtime's list, each item the value it
        // is, however its items differ in kind.
        let lands_as_cell = self.tower_from.borrow()[self.on_words.get() as usize].contains_key(&pc);
        if let (Op::NewList { dst, start, count }, true) = (*op, lands_as_cell) {
            let mut items = Vec::with_capacity(count as usize);
            for i in 0..count {
                items.push(self.tower_reg(start + i, kinds)?);
            }
            let run = if items.is_empty() {
                "0, NULL".to_string()
            } else {
                format!("{count}, (LTower[]){{{}}}", items.join(", "))
            };
            self.set_tower(dst, &format!("ltower_list({run})"));
            return Ok(Some(true));
        }
        if !reads_cell {
            return Ok(None);
        }
        match *op {
            // What only moves a slot (a copy, an argument, a result, a store into a global, a field or a tuple, text
            // a cell is rendered into) the walker's own lowering writes; the conversions a cell needs there are its.
            Op::Move { .. }
            | Op::Return { .. }
            | Op::Call { .. }
            | Op::CallValue { .. }
            | Op::GlobalSet { .. }
            | Op::StructInsert { .. }
            | Op::NewTuple { .. }
            | Op::NewInductive { .. }
            | Op::MakeClosure { .. }
            | Op::Concat { .. }
            | Op::FormatValue { .. }
            | Op::DeepClone { .. }
            | Op::EnsureOwned { .. }
            | Op::ListPushField { .. }
            | Op::NewRange { .. } => return Ok(None),
            // A cell read as the index of a list of the walker's own is narrowed to the `Int` it is (`int_operand`).
            Op::Index { collection, .. } | Op::IndexUnchecked { collection, .. }
                if !t(collection) && !matches!(kind_at(kinds, collection), Kind::Map(_) | Kind::MapUnknown) =>
            {
                return Ok(None)
            }
            Op::SetIndex { collection, value, .. } | Op::SetIndexUnchecked { collection, value, .. }
                if !t(collection)
                    && !t(value)
                    && !matches!(kind_at(kinds, collection), Kind::Map(_) | Kind::MapUnknown) =>
            {
                return Ok(None)
            }
            // A record's field written by name takes a cell as any field does (`field_store_value`), and a map stores
            // one under its key as any value (a map of the extension's values converts the others).
            Op::SetIndex { collection, .. } | Op::SetIndexUnchecked { collection, .. }
                if matches!(
                    kind_at(kinds, collection),
                    Kind::Struct(_) | Kind::StructSynth(_) | Kind::Map(_) | Kind::MapUnknown
                ) =>
            {
                return Ok(None)
            }
            Op::CallBuiltin {
                dst,
                builtin,
                args_start,
                arg_count,
            } if lifts(builtin).is_some() => {
                let f = lifts(builtin).expect("a lifting builtin");
                let mut args = Vec::with_capacity(arg_count as usize);
                for i in 0..arg_count {
                    args.push(self.tower_reg(args_start + i, kinds)?);
                }
                self.set_tower(dst, &format!("ltower_{f}({})", args.join(", ")));
            }
            Op::Add { lhs, rhs, .. } | Op::AddAssign { dst: lhs, src: rhs }
                if kind_at(kinds, lhs) == Kind::Text || kind_at(kinds, rhs) == Kind::Text =>
            {
                return Ok(None);
            }
            Op::AddAssign { dst, src } => {
                let (a, b) = (self.tower_reg(dst, kinds)?, self.tower_reg(src, kinds)?);
                self.set_tower(dst, &format!("ltower_add({a}, {b})"));
            }
            Op::Not { dst, src } => {
                let v = self.tower_reg(src, kinds)?;
                self.set_tower(dst, &format!("ltower_not({v})"));
            }
            Op::Index { dst, collection, index } | Op::IndexUnchecked { dst, collection, index } if t(collection) => {
                let i = self.tower_reg(index, kinds)?;
                self.set_tower(dst, &format!("ltower_item({}, {i})", cell(collection)));
            }
            Op::NewList { dst, start, count } => {
                let mut items = Vec::with_capacity(count as usize);
                for i in 0..count {
                    items.push(self.tower_reg(start + i, kinds)?);
                }
                self.set_tower(dst, &format!("ltower_list({count}, (LTower[]){{{}}})", items.join(", ")));
            }
            Op::ListPush { list, value } if t(list) => {
                self.own_target(list, Kind::Tower)?;
                let v = self.tower_reg(value, kinds)?;
                self.line(&format!("ltower_push({}, {v});", cell(list)));
            }
            Op::ListPop { list, dst } if t(list) => {
                self.own_target(list, Kind::Tower)?;
                self.set_tower(dst, &format!("ltower_pop({})", cell(list)));
            }
            Op::SetIndex { collection, index, value } | Op::SetIndexUnchecked { collection, index, value }
                if t(collection) =>
            {
                self.own_target(collection, Kind::Tower)?;
                let (i, v) = (self.tower_reg(index, kinds)?, self.tower_reg(value, kinds)?);
                self.line(&format!("ltower_set_item({}, {i}, {v});", cell(collection)));
            }
            Op::SliceOp { dst, collection, start, end } if t(collection) => {
                let (from, to) = (self.tower_reg(start, kinds)?, self.tower_reg(end, kinds)?);
                self.set_tower(dst, &format!("ltower_slice({}, {from}, {to})", cell(collection)));
            }
            Op::Length { dst, collection } if t(collection) => {
                self.line(&format!("r[{dst}] = ltower_length({});", cell(collection)));
                return Ok(Some(false));
            }
            Op::Contains { dst, collection, value } if t(collection) => {
                let v = self.tower_reg(value, kinds)?;
                self.line(&format!("r[{dst}] = ltower_contains({}, {v});", cell(collection)));
                return Ok(Some(false));
            }
            Op::Show { src } => self.line(&format!("show_ltower({});", cell(src))),
            Op::JumpIfFalse { cond, target } => {
                self.line(&format!("if (!ltower_truth({})) goto {}{target};", cell(cond), self.label))
            }
            Op::JumpIfTrue { cond, target } => {
                self.line(&format!("if (ltower_truth({})) goto {}{target};", cell(cond), self.label))
            }
            Op::IterPrepare { iterable } if t(iterable) => {
                let id = self.fresh();
                self.line(&format!(
                    "LTower _ti{id} = ltower_items({}); int64_t _tn{id} = ltower_length(_ti{id}); Seq_i64 *_itv{id} = (Seq_i64*)lc_gc_alloc(sizeof(Seq_i64)); *_itv{id} = seq_i64_new(); \
                     for (int64_t _tk = 1; _tk <= _tn{id}; _tk++) seq_i64_push(_itv{id}, (int64_t)(intptr_t)ltower_item_at(_ti{id}, _tk)); size_t _it{id} = 0;",
                    cell(iterable)
                ));
                self.iter_stack.push(super::IterFrame {
                    seq_kind: self.seq_kind_for(Kind::Tower),
                    text_bytes: false,
                    id,
                });
            }
            ref other => match (binary(other), by_constant(other)) {
                (Some((dst, lhs, rhs, f)), _) => {
                    let (a, b) = (self.tower_reg(lhs, kinds)?, self.tower_reg(rhs, kinds)?);
                    self.set_tower(dst, &format!("ltower_{f}({a}, {b})"));
                }
                (None, Some((dst, lhs, divisor, f))) => {
                    let a = self.tower_reg(lhs, kinds)?;
                    self.set_tower(dst, &format!("ltower_{f}({a}, ltower_int((int64_t){divisor}LL))"));
                }
                (None, None) => return Err(format!("pc {pc}: {other:?} over an exact value is not yet lowered")),
            },
        }
        Ok(Some(true))
    }
}
