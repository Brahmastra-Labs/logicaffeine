//! The bytecode dispatch loop.
//!
//! Registers live in one contiguous `Vec<Value>`; `base` is the current frame's
//! offset into it, so every register access is `registers[base + r]`. Calls use
//! register windowing — the callee's frame starts at the caller's `args_start`,
//! so arguments are passed with zero copying.

use super::instruction::{CompiledProgram, Constant, Op, Reg};
use super::value::Value;
use super::MAX_REGISTER_FILE;

struct CallFrame {
    return_pc: usize,
    return_reg: Reg,
    caller_base: usize,
    restore_len: usize,
    /// Iterator-stack depth at call entry; a Return unwinds any iterators the
    /// callee left open (e.g. `Return` inside a `Repeat`).
    iter_depth: usize,
}

pub struct Vm<'p> {
    program: &'p CompiledProgram,
    registers: Vec<Value>,
    base: usize,
    /// One element per `Show` (a shown value may itself contain newlines —
    /// it is still ONE output line, like the tree-walker's emit callback).
    lines: Vec<String>,
    /// Live `Repeat` snapshots: (elements, next index). Stack-disciplined —
    /// `IterPrepare` pushes, `IterPop` pops, nesting nests.
    iter_stack: Vec<(Vec<Value>, usize)>,
    /// Promoted globals (None = not yet defined; reading one is the
    /// "Undefined variable" error).
    globals: Vec<Option<Value>>,
    /// Policy registry + interner for `Check` statements (absent ⇒ the
    /// tree-walker's "Security Check requires policies" error).
    policy_ctx: Option<(&'p crate::analysis::PolicyRegistry, &'p crate::intern::Interner)>,
    /// The pluggable native tier (None = pure bytecode, e.g. WASM).
    tier: Option<&'p dyn super::native_tier::NativeTier>,
    /// Per-function call counts (profiling toward the tier threshold).
    hot: Vec<u32>,
    /// Per-function native state.
    native: Vec<super::native_tier::NativeSlot>,
    /// Back-edge counts for MAIN loops (keyed by loop-head pc).
    region_hot: std::collections::HashMap<usize, u32>,
    /// Compiled Main-loop regions (keyed by loop-head pc).
    regions: std::collections::HashMap<usize, super::native_tier::RegionSlot>,
}

impl<'p> Vm<'p> {
    pub fn new(program: &'p CompiledProgram) -> Self {
        Vm {
            program,
            registers: vec![Value::nothing(); program.register_count],
            base: 0,
            lines: Vec::new(),
            iter_stack: Vec::new(),
            globals: vec![None; program.globals.len()],
            policy_ctx: None,
            tier: None,
            hot: vec![0; program.functions.len()],
            native: (0..program.functions.len())
                .map(|_| super::native_tier::NativeSlot::Untried)
                .collect(),
            region_hot: std::collections::HashMap::new(),
            regions: std::collections::HashMap::new(),
        }
    }

    /// Back-edge hook for MAIN loops (`Jump` to an earlier pc at Main depth):
    /// profile, compile when hot, and — when ready and the guard passes — run
    /// the region natively. Returns the pc to resume at (the loop's exit).
    fn try_region(&mut self, head: usize, back_pc: usize) -> Option<usize> {
        use super::native_tier::{RegionSlot, REGION_TIER_THRESHOLD};
        let tier = self.tier?;
        match self.regions.get(&head) {
            Some(RegionSlot::Failed) => return None,
            Some(RegionSlot::Ready { .. }) => {}
            None => {
                let n = self.region_hot.entry(head).or_insert(0);
                *n += 1;
                if *n < REGION_TIER_THRESHOLD {
                    return None;
                }
                // Region extent: every jump leaving [head, back_pc] must
                // agree on ONE exit pc.
                let body = &self.program.code[head..=back_pc];
                let mut exit: Option<usize> = None;
                for op in body {
                    if let Op::Jump { target } | Op::JumpIfFalse { target, .. }
                    | Op::JumpIfTrue { target, .. } = *op
                    {
                        if !(head..=back_pc).contains(&target) {
                            match exit {
                                None => exit = Some(target),
                                Some(e) if e == target => {}
                                _ => {
                                    self.regions.insert(head, RegionSlot::Failed);
                                    return None;
                                }
                            }
                        }
                    }
                }
                let Some(exit_pc) = exit else {
                    self.regions.insert(head, RegionSlot::Failed);
                    return None;
                };
                let reg_count = u16::try_from(self.program.register_count).ok()?;
                match tier.compile_region(body, head, exit_pc, &self.program.constants, reg_count)
                {
                    Some(rf) => {
                        self.regions.insert(head, RegionSlot::Ready { rf, exit_pc });
                    }
                    None => {
                        self.regions.insert(head, RegionSlot::Failed);
                        return None;
                    }
                }
            }
        }
        let Some(RegionSlot::Ready { rf, exit_pc }) = self.regions.get(&head) else {
            unreachable!()
        };
        // Guard: every live-in slot must currently be an Int.
        for &r in rf.guard_set() {
            self.registers.get(self.base + r as usize)?.as_int()?;
        }
        let mut frame = vec![0i64; rf.frame_size()];
        for &r in rf.guard_set() {
            frame[r as usize] = self.registers[self.base + r as usize].as_int().unwrap();
        }
        rf.run(&mut frame);
        let writes: Vec<(u16, bool)> = rf.write_set().to_vec();
        let exit = *exit_pc;
        for (r, is_bool) in writes {
            let v = if is_bool { Value::bool(frame[r as usize] != 0) } else { Value::int(frame[r as usize]) };
            self.set(r, v);
        }
        Some(exit)
    }

    /// Install a native tier: hot functions in the integer subset run as
    /// JIT-compiled machine code, guarded per call (non-Int args deopt to
    /// the bytecode path).
    pub fn with_native_tier(mut self, tier: &'p dyn super::native_tier::NativeTier) -> Self {
        self.tier = Some(tier);
        self
    }

    /// Tier dispatch for `Call`: Some(result) = the native fast path ran.
    fn try_native(
        &mut self,
        func: u16,
        args_start: super::instruction::Reg,
        arg_count: u16,
    ) -> Option<Value> {
        use super::native_tier::{NativeSlot, NATIVE_TIER_THRESHOLD};
        let tier = self.tier?;
        let fi = func as usize;
        if matches!(self.native.get(fi)?, NativeSlot::Failed) {
            return None;
        }
        if matches!(self.native[fi], NativeSlot::Untried) {
            self.hot[fi] += 1;
            if self.hot[fi] < NATIVE_TIER_THRESHOLD {
                return None;
            }
            let f = &self.program.functions[fi];
            if !f.captures.is_empty() {
                self.native[fi] = NativeSlot::Failed;
                return None;
            }
            let end = self
                .program
                .functions
                .iter()
                .map(|g| g.entry_pc)
                .filter(|&e| e > f.entry_pc)
                .min()
                .unwrap_or(self.program.code.len());
            match tier.compile_function(
                &self.program.code[f.entry_pc..end],
                f.entry_pc,
                &self.program.constants,
                f.param_count,
                f.register_count as u16,
            ) {
                Some(nf) => self.native[fi] = NativeSlot::Ready(nf),
                None => {
                    self.native[fi] = NativeSlot::Failed;
                    return None;
                }
            }
        }
        // The per-call guard: every argument must be an Int, else deopt.
        let base = self.base + args_start as usize;
        let mut args = Vec::with_capacity(arg_count as usize);
        for k in 0..arg_count as usize {
            args.push(self.registers.get(base + k)?.as_int()?);
        }
        let NativeSlot::Ready(nf) = &self.native[fi] else { unreachable!() };
        let raw = nf.call(&args);
        Some(if nf.returns_bool() { Value::bool(raw != 0) } else { Value::int(raw) })
    }

    /// Provide the policy registry (and the interner its symbols live in) for
    /// `Check` statements.
    pub fn with_policy_ctx(
        mut self,
        registry: &'p crate::analysis::PolicyRegistry,
        interner: &'p crate::intern::Interner,
    ) -> Self {
        self.policy_ctx = Some((registry, interner));
        self
    }

    /// The output lines, one per `Show`.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The output as one string (one trailing newline per `Show`).
    pub fn output(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    /// Consume the VM, returning its output lines.
    pub fn into_lines(self) -> Vec<String> {
        self.lines
    }

    pub fn run(&mut self) -> Result<(), String> {
        let mut pc = 0usize;
        let mut call_stack: Vec<CallFrame> = Vec::new();

        while pc < self.program.code.len() {
            match self.program.code[pc].clone() {
                Op::LoadConst { dst, idx } => {
                    let v = const_to_value(&self.program.constants[idx as usize]);
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Move { dst, src } => {
                    self.set(dst, self.reg(src).clone());
                    pc += 1;
                }
                Op::Add { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::add)?; pc += 1; }
                Op::Sub { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::sub)?; pc += 1; }
                Op::Mul { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::mul)?; pc += 1; }
                Op::Div { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::div)?; pc += 1; }
                Op::Mod { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::modulo)?; pc += 1; }
                Op::Lt { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::lt)?; pc += 1; }
                Op::Gt { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::gt)?; pc += 1; }
                Op::LtEq { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::lte)?; pc += 1; }
                Op::GtEq { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::gte)?; pc += 1; }
                Op::Eq { dst, lhs, rhs } => {
                    let v = self.reg(lhs).eq_op(self.reg(rhs));
                    self.set(dst, v);
                    pc += 1;
                }
                Op::NotEq { dst, lhs, rhs } => {
                    let v = self.reg(lhs).neq_op(self.reg(rhs));
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Not { dst, src } => {
                    let v = self.reg(src).not_op()?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::AndEager { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::and_eager)?; pc += 1; }
                Op::OrEager { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::or_eager)?; pc += 1; }
                Op::Concat { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::concat)?; pc += 1; }
                Op::BitXor { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::bitxor)?; pc += 1; }
                Op::Shl { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::shl)?; pc += 1; }
                Op::Shr { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::shr)?; pc += 1; }
                Op::Jump { target } => {
                    // A back-edge at Main depth is a hot-loop candidate.
                    if target < pc && call_stack.is_empty() && self.tier.is_some() {
                        if let Some(exit) = self.try_region(target, pc) {
                            pc = exit;
                            continue;
                        }
                    }
                    pc = target;
                }
                Op::JumpIfFalse { cond, target } => {
                    if !self.reg(cond).is_truthy() { pc = target; } else { pc += 1; }
                }
                Op::JumpIfTrue { cond, target } => {
                    if self.reg(cond).is_truthy() { pc = target; } else { pc += 1; }
                }
                Op::JumpIfInt { cond, target } => {
                    if self.reg(cond).is_int() { pc = target; } else { pc += 1; }
                }
                Op::GlobalGet { dst, idx } => {
                    match &self.globals[idx as usize] {
                        Some(v) => {
                            let v = v.clone();
                            self.set(dst, v);
                        }
                        None => {
                            return Err(format!(
                                "Undefined variable: {}",
                                self.program.globals[idx as usize]
                            ));
                        }
                    }
                    pc += 1;
                }
                Op::GlobalSet { idx, src } => {
                    self.globals[idx as usize] = Some(self.reg(src).clone());
                    pc += 1;
                }
                Op::MakeClosure { dst, func, locals_start } => {
                    use crate::interpreter::{ClosureValue, RuntimeValue};
                    let f = self
                        .program
                        .functions
                        .get(func as usize)
                        .ok_or("vm: MakeClosure on undefined function index")?;
                    let mut captured_env = std::collections::HashMap::new();
                    let mut local_k: Reg = 0;
                    for (sym, global_idx) in &f.captures {
                        match global_idx {
                            Some(gidx) => {
                                // Snapshot the global IF it is defined; an
                                // undefined one is simply not captured — the
                                // body falls through to the live global.
                                if let Some(v) = &self.globals[*gidx as usize] {
                                    captured_env.insert(*sym, v.as_runtime().deep_clone());
                                }
                            }
                            None => {
                                let v = self.reg(locals_start + local_k).as_runtime().deep_clone();
                                captured_env.insert(*sym, v);
                                local_k += 1;
                            }
                        }
                    }
                    let param_names = vec![crate::intern::Symbol::default(); f.param_count as usize];
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Function(Box::new(ClosureValue {
                            body_index: func as usize,
                            captured_env,
                            param_names,
                        }))),
                    );
                    pc += 1;
                }
                Op::CallValue { dst, callee, args_start, arg_count, name_for_err } => {
                    use crate::interpreter::RuntimeValue;
                    let closure = match self.reg(callee).as_runtime() {
                        RuntimeValue::Function(c) => (**c).clone(),
                        other => {
                            return Err(if name_for_err == u32::MAX {
                                format!("Cannot call value of type {}", other.type_name())
                            } else {
                                match &self.program.constants[name_for_err as usize] {
                                    Constant::Text(n) => format!("Unknown function: {}", n),
                                    _ => format!("Cannot call value of type {}", other.type_name()),
                                }
                            });
                        }
                    };
                    if call_stack.len() >= crate::semantics::MAX_CALL_DEPTH {
                        return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
                    }
                    let f = self
                        .program
                        .functions
                        .get(closure.body_index)
                        .ok_or("vm: CallValue on undefined function index")?;
                    if arg_count as usize != f.param_count as usize {
                        return Err(format!(
                            "Closure expects {} arguments, got {}",
                            f.param_count, arg_count
                        ));
                    }
                    let entry_pc = f.entry_pc;
                    let reg_count = f.register_count;
                    let captures = f.captures.clone();
                    let param_count = f.param_count;

                    let callee_base = self.base + args_start as usize;
                    let restore_len = self.registers.len();
                    let needed = callee_base + reg_count;
                    if needed > MAX_REGISTER_FILE {
                        return Err("vm: register file limit exceeded".to_string());
                    }
                    if self.registers.len() < needed {
                        self.registers.resize(needed, Value::nothing());
                    }
                    call_stack.push(CallFrame {
                        return_pc: pc + 1,
                        return_reg: dst,
                        caller_base: self.base,
                        restore_len,
                        iter_depth: self.iter_stack.len(),
                    });
                    self.base = callee_base;
                    // Bind captures: value slots then present flags — both
                    // deep-cloned PER CALL (the tree-walker re-clones each
                    // invocation).
                    let cap_count = captures.len() as Reg;
                    for (k, (sym, _)) in captures.iter().enumerate() {
                        let (v, present) = match closure.captured_env.get(sym) {
                            Some(v) => (Value::from_runtime(v.deep_clone()), true),
                            None => (Value::nothing(), false),
                        };
                        self.set(param_count + k as Reg, v);
                        self.set(param_count + cap_count + k as Reg, Value::bool(present));
                    }
                    pc = entry_pc;
                }
                Op::CallBuiltin { dst, builtin, args_start, arg_count } => {
                    let mut args = Vec::with_capacity(arg_count as usize);
                    for k in 0..arg_count {
                        args.push(self.reg(args_start + k).as_runtime().clone());
                    }
                    let v = crate::semantics::builtins::call_builtin(builtin, args)?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::Call { dst, func, args_start, arg_count } => {
                    if call_stack.len() >= crate::semantics::MAX_CALL_DEPTH {
                        return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
                    }
                    if let Some(v) = self.try_native(func, args_start, arg_count) {
                        self.set(dst, v);
                        pc += 1;
                        continue;
                    }
                    let (entry_pc, reg_count) = {
                        let f = self
                            .program
                            .functions
                            .get(func as usize)
                            .ok_or("vm: call to undefined function index")?;
                        (f.entry_pc, f.register_count)
                    };
                    let callee_base = self.base + args_start as usize;
                    let restore_len = self.registers.len();
                    let needed = callee_base + reg_count;
                    if needed > MAX_REGISTER_FILE {
                        return Err("vm: register file limit exceeded".to_string());
                    }
                    if self.registers.len() < needed {
                        self.registers.resize(needed, Value::nothing());
                    }
                    call_stack.push(CallFrame {
                        return_pc: pc + 1,
                        return_reg: dst,
                        caller_base: self.base,
                        restore_len,
                        iter_depth: self.iter_stack.len(),
                    });
                    self.base = callee_base;
                    pc = entry_pc;
                }
                Op::Return { src } => {
                    let frame = call_stack.pop().ok_or("vm: return with no caller")?;
                    let rv = self.reg(src).clone();
                    self.iter_stack.truncate(frame.iter_depth);
                    self.registers.truncate(frame.restore_len);
                    self.base = frame.caller_base;
                    let slot = self.base + frame.return_reg as usize;
                    self.registers[slot] = rv;
                    pc = frame.return_pc;
                }
                Op::ReturnNothing => {
                    let frame = call_stack.pop().ok_or("vm: return with no caller")?;
                    self.iter_stack.truncate(frame.iter_depth);
                    self.registers.truncate(frame.restore_len);
                    self.base = frame.caller_base;
                    let slot = self.base + frame.return_reg as usize;
                    self.registers[slot] = Value::nothing();
                    pc = frame.return_pc;
                }
                Op::NewList { dst, start, count } => {
                    let mut items = Vec::with_capacity(count as usize);
                    for k in 0..count {
                        items.push(self.reg(start + k).clone());
                    }
                    self.set(dst, Value::list(items));
                    pc += 1;
                }
                Op::NewEmptyList { dst } => { self.set(dst, Value::empty_list()); pc += 1; }
                Op::NewEmptySet { dst } => { self.set(dst, Value::empty_set()); pc += 1; }
                Op::NewEmptyMap { dst } => { self.set(dst, Value::empty_map()); pc += 1; }
                Op::NewRange { dst, start, end } => {
                    let (lo, hi) = match (self.reg(start).as_int(), self.reg(end).as_int()) {
                        (Some(lo), Some(hi)) => (lo, hi),
                        _ => return Err("Range requires Int bounds".to_string()),
                    };
                    self.set(dst, Value::int_range(lo, hi));
                    pc += 1;
                }
                Op::ListPush { list, value } => {
                    let v = self.reg(value).clone();
                    self.reg(list).list_push(v)?;
                    pc += 1;
                }
                Op::SetAdd { set, value } => {
                    let v = self.reg(value).clone();
                    self.reg(set).set_add(v)?;
                    pc += 1;
                }
                Op::RemoveFrom { collection, value } => {
                    self.reg(collection).remove_from(self.reg(value))?;
                    pc += 1;
                }
                Op::SetIndex { collection, index, value } => {
                    use crate::interpreter::RuntimeValue;
                    // Struct field set via index syntax (`Set item "f" of s to
                    // v`) — VALUE semantics, mirroring the tree-walker: clone
                    // the struct, insert, write the new struct back.
                    let is_struct_text = matches!(
                        (self.reg(collection).as_runtime(), self.reg(index).as_runtime()),
                        (RuntimeValue::Struct(_), RuntimeValue::Text(_))
                    );
                    if is_struct_text {
                        let field = match self.reg(index).as_runtime() {
                            RuntimeValue::Text(t) => t.to_string(),
                            _ => unreachable!(),
                        };
                        let new_val = self.reg(value).as_runtime().clone();
                        let target = self.reg_mut(collection);
                        match target.as_runtime_mut() {
                            RuntimeValue::Struct(st) => {
                                st.fields.insert(field, new_val);
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        let v = self.reg(value).clone();
                        self.reg(collection).index_set(self.reg(index), v)?;
                    }
                    pc += 1;
                }
                Op::Index { dst, collection, index } => {
                    let v = self.reg(collection).index_get(self.reg(index))?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Length { dst, collection } => {
                    let n = self.reg(collection).len()?;
                    self.set(dst, Value::int(n));
                    pc += 1;
                }
                Op::Contains { dst, collection, value } => {
                    let b = self.reg(collection).contains(self.reg(value))?;
                    self.set(dst, Value::bool(b));
                    pc += 1;
                }
                Op::ListPushField { obj, field, src } => {
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    let val = self.reg(src).as_runtime().clone();
                    crate::semantics::collections::push_to_struct_field(
                        self.reg(obj).as_runtime(),
                        &field_name,
                        val,
                    )?;
                    pc += 1;
                }
                Op::CheckPolicy { subject, predicate, is_capability, object, source_text } => {
                    let (registry, interner) = match self.policy_ctx {
                        Some(ctx) => ctx,
                        None => {
                            return Err(
                                "Security Check requires policies. Use compiled Rust or add ## Policy block."
                                    .to_string(),
                            );
                        }
                    };
                    let source = match &self.program.constants[source_text as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: check source is not Text: {:?}", other)),
                    };
                    let obj_val = if object != Reg::MAX {
                        Some(self.reg(object).as_runtime().clone())
                    } else {
                        None
                    };
                    crate::semantics::policy::check_policy(
                        registry,
                        interner,
                        self.reg(subject).as_runtime(),
                        predicate,
                        is_capability,
                        obj_val.as_ref(),
                        &source,
                    )?;
                    pc += 1;
                }
                Op::FormatValue { dst, src, spec, debug_prefix } => {
                    let mut out = String::new();
                    if debug_prefix != u32::MAX {
                        match &self.program.constants[debug_prefix as usize] {
                            Constant::Text(p) => {
                                out.push_str(p);
                                out.push('=');
                            }
                            other => {
                                return Err(format!("vm: debug prefix is not Text: {:?}", other));
                            }
                        }
                    }
                    if spec != u32::MAX {
                        let spec_s = match &self.program.constants[spec as usize] {
                            Constant::Text(s) => s.as_str(),
                            other => return Err(format!("vm: format spec is not Text: {:?}", other)),
                        };
                        out.push_str(&crate::semantics::format::apply_format_spec(
                            self.reg(src).as_runtime(),
                            spec_s,
                        ));
                    } else {
                        out.push_str(&self.reg(src).to_display_string());
                    }
                    self.set(dst, Value::text(out));
                    pc += 1;
                }
                Op::SliceOp { dst, collection, start, end } => {
                    let v = crate::semantics::collections::slice(
                        self.reg(collection).as_runtime(),
                        self.reg(start).as_runtime(),
                        self.reg(end).as_runtime(),
                    )?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::DeepClone { dst, src } => {
                    let v = self.reg(src).as_runtime().deep_clone();
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::NewTuple { dst, start, count } => {
                    use crate::interpreter::RuntimeValue;
                    let mut items = Vec::with_capacity(count as usize);
                    for k in 0..count {
                        items.push(self.reg(start + k).as_runtime().clone());
                    }
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Tuple(std::rc::Rc::new(items))),
                    );
                    pc += 1;
                }
                Op::UnionOp { dst, lhs, rhs } => {
                    let v = crate::semantics::collections::union(
                        self.reg(lhs).as_runtime(),
                        self.reg(rhs).as_runtime(),
                    )?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::IntersectOp { dst, lhs, rhs } => {
                    let v = crate::semantics::collections::intersection(
                        self.reg(lhs).as_runtime(),
                        self.reg(rhs).as_runtime(),
                    )?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::LoadToday { dst } => {
                    self.set(dst, Value::from_runtime(crate::semantics::temporal::today()));
                    pc += 1;
                }
                Op::LoadNow { dst } => {
                    self.set(dst, Value::from_runtime(crate::semantics::temporal::now()));
                    pc += 1;
                }
                Op::NewStruct { dst, type_name } => {
                    use crate::interpreter::{RuntimeValue, StructValue};
                    let name = match &self.program.constants[type_name as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: NewStruct name is not Text: {:?}", other)),
                    };
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Struct(Box::new(StructValue {
                            type_name: name,
                            fields: std::collections::HashMap::new(),
                        }))),
                    );
                    pc += 1;
                }
                Op::StructInsert { obj, field, value } => {
                    use crate::interpreter::RuntimeValue;
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    let v = self.reg(value).as_runtime().clone();
                    match self.reg_mut(obj).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            s.fields.insert(field_name, v);
                        }
                        _ => return Err("Cannot set field on non-struct value".to_string()),
                    }
                    pc += 1;
                }
                Op::GetField { dst, obj, field } => {
                    use crate::interpreter::RuntimeValue;
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.as_str(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    let v = match self.reg(obj).as_runtime() {
                        RuntimeValue::Struct(s) => s
                            .fields
                            .get(field_name)
                            .cloned()
                            .ok_or_else(|| format!("Field '{}' not found", field_name))?,
                        other => {
                            return Err(format!("Cannot access field on {}", other.type_name()));
                        }
                    };
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::NewInductive { dst, type_name, ctor, args_start, count } => {
                    use crate::interpreter::{InductiveValue, RuntimeValue};
                    let inductive_type = match &self.program.constants[type_name as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: enum name is not Text: {:?}", other)),
                    };
                    let constructor = match &self.program.constants[ctor as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: variant name is not Text: {:?}", other)),
                    };
                    let mut args = Vec::with_capacity(count as usize);
                    for k in 0..count {
                        args.push(self.reg(args_start + k).as_runtime().clone());
                    }
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Inductive(Box::new(InductiveValue {
                            inductive_type,
                            constructor,
                            args,
                        }))),
                    );
                    pc += 1;
                }
                Op::TestArm { dst, target, variant } => {
                    use crate::interpreter::RuntimeValue;
                    let variant_name = match &self.program.constants[variant as usize] {
                        Constant::Text(s) => s.as_str(),
                        other => return Err(format!("vm: variant name is not Text: {:?}", other)),
                    };
                    let matched = match self.reg(target).as_runtime() {
                        RuntimeValue::Struct(s) => s.type_name == variant_name,
                        RuntimeValue::Inductive(ind) => ind.constructor == variant_name,
                        _ => false,
                    };
                    self.set(dst, Value::bool(matched));
                    pc += 1;
                }
                Op::BindArm { dst, target, field, index } => {
                    use crate::interpreter::RuntimeValue;
                    let v = match self.reg(target).as_runtime() {
                        RuntimeValue::Struct(s) => {
                            let field_name = match &self.program.constants[field as usize] {
                                Constant::Text(s) => s.as_str(),
                                other => {
                                    return Err(format!("vm: field name is not Text: {:?}", other));
                                }
                            };
                            s.fields.get(field_name).cloned()
                        }
                        RuntimeValue::Inductive(ind) => ind.args.get(index as usize).cloned(),
                        _ => None,
                    };
                    if let Some(v) = v {
                        self.set(dst, Value::from_runtime(v));
                    }
                    pc += 1;
                }
                Op::CrdtBump { obj, field, amount, negate } => {
                    use crate::interpreter::RuntimeValue;
                    let amount_int = match self.reg(amount).as_runtime() {
                        RuntimeValue::Int(n) => *n,
                        _ => {
                            return Err(if negate {
                                "CRDT decrement amount must be an integer".to_string()
                            } else {
                                "CRDT increment amount must be an integer".to_string()
                            });
                        }
                    };
                    let amount_int = if negate { amount_int.wrapping_neg() } else { amount_int };
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    match self.reg_mut(obj).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            let current =
                                s.fields.get(&field_name).cloned().unwrap_or(RuntimeValue::Int(0));
                            let new_val = crate::semantics::arith::crdt_counter_bump(
                                current,
                                amount_int,
                                &field_name,
                            )?;
                            s.fields.insert(field_name, new_val);
                        }
                        _ => {
                            return Err(if negate {
                                "Cannot decrease field on non-struct value".to_string()
                            } else {
                                "Cannot increase field on non-struct value".to_string()
                            });
                        }
                    }
                    pc += 1;
                }
                Op::CrdtMerge { target, source } => {
                    use crate::interpreter::RuntimeValue;
                    let source_fields = match self.reg(source).as_runtime() {
                        RuntimeValue::Struct(s) => s.fields.clone(),
                        _ => return Err("Merge source must be a struct".to_string()),
                    };
                    match self.reg_mut(target).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            for (field_name, incoming) in source_fields {
                                let current =
                                    s.fields.get(&field_name).cloned().unwrap_or(RuntimeValue::Int(0));
                                let merged =
                                    crate::semantics::arith::crdt_merge_field(&current, incoming);
                                s.fields.insert(field_name, merged);
                            }
                        }
                        _ => return Err("Merge target must be a struct".to_string()),
                    }
                    pc += 1;
                }
                Op::IterPrepare { iterable } => {
                    let items = crate::semantics::collections::iteration_snapshot(
                        self.reg(iterable).as_runtime(),
                    )?;
                    self.iter_stack
                        .push((items.into_iter().map(Value::from_runtime).collect(), 0));
                    pc += 1;
                }
                Op::IterNext { dst, exit } => {
                    let (items, idx) = self
                        .iter_stack
                        .last_mut()
                        .ok_or("vm: IterNext with no live iterator")?;
                    if *idx < items.len() {
                        let v = items[*idx].clone();
                        *idx += 1;
                        self.set(dst, v);
                        pc += 1;
                    } else {
                        pc = exit;
                    }
                }
                Op::IterPop => {
                    self.iter_stack.pop().ok_or("vm: IterPop with no live iterator")?;
                    pc += 1;
                }
                Op::ListPop { list, dst } => {
                    let v = crate::semantics::collections::list_pop(self.reg(list).as_runtime())?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::Sleep { duration } => {
                    use crate::interpreter::RuntimeValue;
                    let nanos = match self.reg(duration).as_runtime() {
                        RuntimeValue::Duration(nanos) => *nanos,
                        RuntimeValue::Int(ms) => ms.wrapping_mul(1_000_000),
                        other => {
                            return Err(format!(
                                "Sleep requires Duration or Int, got {}",
                                other.type_name()
                            ));
                        }
                    };
                    if nanos > 0 {
                        #[cfg(not(target_arch = "wasm32"))]
                        std::thread::sleep(std::time::Duration::from_nanos(nanos as u64));
                        #[cfg(target_arch = "wasm32")]
                        return Err("Sleep requires async execution path".to_string());
                    }
                    pc += 1;
                }
                Op::DestructureTuple { src, start, count } => {
                    use crate::interpreter::RuntimeValue;
                    match self.reg(src).as_runtime() {
                        RuntimeValue::Tuple(items) => {
                            // Zip semantics: bind up to the shorter side.
                            let items: Vec<Value> = items
                                .iter()
                                .take(count as usize)
                                .cloned()
                                .map(Value::from_runtime)
                                .collect();
                            for (i, v) in items.into_iter().enumerate() {
                                self.set(start + i as Reg, v);
                            }
                        }
                        other => {
                            return Err(format!(
                                "Expected tuple for pattern, got {}",
                                other.type_name()
                            ));
                        }
                    }
                    pc += 1;
                }
                Op::Show { src } => {
                    self.lines.push(self.reg(src).to_display_string());
                    pc += 1;
                }
                Op::FailWith { msg } => {
                    return Err(match &self.program.constants[msg as usize] {
                        Constant::Text(s) => s.clone(),
                        other => format!("vm: FailWith constant is not Text: {:?}", other),
                    });
                }
                Op::Halt => break,
            }
        }
        Ok(())
    }

    #[inline]
    fn reg(&self, r: Reg) -> &Value {
        &self.registers[self.base + r as usize]
    }

    #[inline]
    fn set(&mut self, r: Reg, v: Value) {
        let slot = self.base + r as usize;
        self.registers[slot] = v;
    }

    #[inline]
    fn reg_mut(&mut self, r: Reg) -> &mut Value {
        let slot = self.base + r as usize;
        &mut self.registers[slot]
    }

    fn binop(
        &mut self,
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
        f: impl Fn(&Value, &Value) -> Result<Value, String>,
    ) -> Result<(), String> {
        let v = f(self.reg(lhs), self.reg(rhs))?;
        self.set(dst, v);
        Ok(())
    }
}

fn const_to_value(c: &Constant) -> Value {
    use crate::interpreter::RuntimeValue;
    match c {
        Constant::Int(n) => Value::int(*n),
        Constant::Float(f) => Value::float(*f),
        Constant::Bool(b) => Value::bool(*b),
        Constant::Text(s) => Value::text(s.clone()),
        Constant::Char(c) => Value::from_runtime(RuntimeValue::Char(*c)),
        Constant::Nothing => Value::nothing(),
        Constant::Duration(n) => Value::from_runtime(RuntimeValue::Duration(*n)),
        Constant::Date(d) => Value::from_runtime(RuntimeValue::Date(*d)),
        Constant::Moment(n) => Value::from_runtime(RuntimeValue::Moment(*n)),
        Constant::Span { months, days } => {
            Value::from_runtime(RuntimeValue::Span { months: *months, days: *days })
        }
        Constant::Time(n) => Value::from_runtime(RuntimeValue::Time(*n)),
    }
}
