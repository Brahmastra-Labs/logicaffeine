# LOGOS Bytecode VM — Technical Specification

## Pipeline

```
Source → Lexer → Parser → AST → Resolver → BytecodeCompiler → CompiledProgram → VM → Output
                                  (new)       (modified)                          (new)
```

The VM replaces the sync path only. The async tree-walker stays for I/O-bound programs (`needs_async()` = true). Escape blocks, networking, pipes, agents, and persistence remain compile-to-Rust only.

### Resolver (`resolver.rs`)

The Resolver walks the AST before compilation and assigns every variable a `Location`:

```rust
enum Location {
    Local(Reg),     // Stack-relative register index within the current frame
    Global(Symbol), // Top-level variable, requires HashMap lookup at runtime
    Upvalue(u8),    // Captured by a closure (pending language feature)
}
```

**Purpose**: Eliminate all runtime name lookups for local variables. The compiler uses Resolver output to emit direct register references instead of `LoadVar`/`SetVar` with symbol names.

**Output**: Side-table `HashMap<ScopeId, ScopeInfo>` where each `ScopeInfo` contains:
- `locals: HashMap<Symbol, Reg>` — variable-to-register mapping
- `register_count: u16` — total registers needed for this scope (determines `CompiledFunction.register_count`)

**What it calculates**:
- For each scope, how many locals exist (determines register allocation)
- For each variable reference, whether it's a local (register), global (name lookup), or upvalue (captured)
- Register reuse across non-overlapping scopes (sibling if/else branches can share registers)

**Existing pattern**: `RefinementContext` in `codegen.rs` already tracks scopes with `Vec<HashMap<Symbol, ...>>` and `push_scope()`/`pop_scope()` — the Resolver adapts this pattern for register assignment.

**File**: `vm/resolver.rs` (~150 lines)

## Module Layout

```
crates/logicaffeine_compile/src/vm/
├── mod.rs           Public API: compile_and_run(), VmError
├── value.rs         Value newtype over RuntimeValue
├── instruction.rs   Op enum, CompiledProgram, CompiledFunction, Constant
├── resolver.rs      Scope resolution: Symbol → Local(Reg) | Global | Upvalue(slot)
├── compiler.rs      BytecodeCompiler: AST → bytecode, register allocation, type tracking
├── machine.rs       VM struct, dispatch loop, CallFrame, register windowing
└── sourcemap.rs     BytecodeSourceMap: instruction offset → Span
```

## Integration Point

`crates/logicaffeine_compile/src/ui_bridge.rs` — `interpret_for_ui_sync()`:

```rust
if needs_async(&stmts) {
    block_on(interp.run(&stmts))       // existing async tree-walker
} else {
    let program = BytecodeCompiler::compile(&stmts, &interner)?;
    let mut vm = VM::new(&program);
    vm.run()?;
}
```

---

## Type Aliases

```rust
type Reg       = u8;     // Register index. 256 registers per frame.
type ConstIdx  = u16;    // Index into CompiledProgram.constants
type StrIdx    = u16;    // Index into CompiledProgram.string_table
type FuncIdx   = u16;    // Index into CompiledProgram.functions
type BuiltinId = u8;     // Builtin function identifier
type IterSlot  = u8;     // Iterator slot (0-7)
```

---

## Value Abstraction (`value.rs`)

```rust
pub struct Value(RuntimeValue);
```

All VM and compiler code accesses values through `Value` methods. No code outside `value.rs` matches on `RuntimeValue` directly. This boundary enables a future swap to NaN-boxed `u64` (8 bytes, 2x cache density) without touching compiler or dispatch code.

### Construction

| Method | Result |
|---|---|
| `Value::int(n: i64)` | `Int(n)` |
| `Value::float(f: f64)` | `Float(f)` |
| `Value::bool(b: bool)` | `Bool(b)` |
| `Value::text(s: String)` | `Text(Rc::new(s))` |
| `Value::char(c: char)` | `Char(c)` |
| `Value::nothing()` | `Nothing` |
| `Value::list(items: Vec<Value>)` | `List(Rc::new(RefCell::new(...)))` |
| `Value::tuple(items: Vec<Value>)` | `Tuple(Rc::new(...))` |
| `Value::set(items: Vec<Value>)` | `Set(Rc::new(RefCell::new(...)))` |
| `Value::map(entries: HashMap<...>)` | `Map(Rc::new(RefCell::new(...)))` |
| `Value::duration(nanos: i64)` | `Duration(nanos)` |
| `Value::date(days: i32)` | `Date(days)` |
| `Value::moment(nanos: i64)` | `Moment(nanos)` |
| `Value::span(months: i32, days: i32)` | `Span { months, days }` |
| `Value::time(nanos: i64)` | `Time(nanos)` |
| `Value::strukt(name: String, fields: HashMap<String, Value>)` | `Struct(Box::new(StructValue { ... }))` |
| `Value::inductive(type_name: String, ctor: String, args: Vec<Value>)` | `Inductive(Box::new(InductiveValue { ... }))` |

### Type Checks

| Method | Returns |
|---|---|
| `is_int() → bool` | true if `Int(_)` |
| `is_float() → bool` | true if `Float(_)` |
| `is_bool() → bool` | true if `Bool(_)` |
| `is_text() → bool` | true if `Text(_)` |
| `is_nothing() → bool` | true if `Nothing` |
| `is_list() → bool` | true if `List(_)` |
| `is_truthy() → bool` | `Bool(b)` → b, `Int(n)` → n != 0, `Nothing` → false, else → true |
| `type_name() → &str` | `"Int"`, `"Float"`, `"Bool"`, `"Text"`, `"List"`, `"Struct"`, etc. |

### Extraction (panics on wrong type)

| Method | Returns |
|---|---|
| `as_int() → i64` | |
| `as_float() → f64` | |
| `as_bool() → bool` | |
| `as_text() → &str` | |

### Collection Operations (mutate in place via Rc<RefCell>)

| Method | Effect |
|---|---|
| `list_push(val: Value)` | Append to list |
| `list_pop() → Value` | Remove last, return Nothing if empty |
| `index_get(idx: &Value) → Result<Value>` | 1-indexed for List/Tuple/Text, key for Map |
| `index_set(idx: &Value, val: Value) → Result<()>` | 1-indexed for List, key for Map |
| `map_get(key: &Value) → Result<Value>` | |
| `map_set(key: Value, val: Value)` | |
| `set_add(val: &Value)` | No-op if already present |
| `set_remove(val: &Value)` | |
| `len() → usize` | List, Tuple, Set, Map, Text |
| `contains(val: &Value) → bool` | List, Set, Map (key), Text (substring) |

### Other

| Method | Effect |
|---|---|
| `to_display_string() → String` | Human-readable output (matches existing `RuntimeValue::to_display_string`) |
| `values_equal(other: &Value) → bool` | Structural equality (matches existing `values_equal`) |
| `deep_clone() → Value` | Recursive deep copy of collections and structs |
| `from_runtime(rv: RuntimeValue) → Value` | Wrap |
| `into_runtime(self) → RuntimeValue` | Unwrap |

---

## Instruction Set (87 opcodes)

Every instruction is a variant of `#[repr(u8)] pub enum Op`. Instructions marked `(+ic)` carry an `ic: u32` inline cache slot, set to 0 in v1. Instructions are listed with their fields and execution semantics.

### Loads

#### `LoadConst { dst: Reg, idx: ConstIdx }`
Load constant from the constant pool into register.
```
R[dst] = Value::from_runtime(program.constants[idx].as_value().clone())
```

#### `Move { dst: Reg, src: Reg }`
Copy value between registers. Shallow copy (Rc clone for collections, not deep clone).
```
R[dst] = R[src].clone()
```

#### `LoadNothing { dst: Reg }`
```
R[dst] = Value::nothing()
```

#### `LoadTrue { dst: Reg }`
```
R[dst] = Value::bool(true)
```

#### `LoadFalse { dst: Reg }`
```
R[dst] = Value::bool(false)
```

### Variables

Local variables are registers. The Resolver assigns each local a register index at compile time. Only top-level globals require name-based lookup at runtime.

- `Let x be 5.` where x is local → compiler assigns x to register 4, emits `LoadConst { dst: 4, idx: ... }`
- `Show x.` where x is local → compiler knows x is register 4, emits `Show { src: 4 }` directly
- `Set x to 10.` where x is local → compiler emits `LoadConst { dst: 4, idx: ... }` (overwrite register)

No name lookup at runtime for locals. The Resolver already determined the register index.

#### `LoadGlobal { dst: Reg, name: ConstIdx }`
Load a top-level global variable by name. HashMap lookup — used for variables defined at module scope only.
```
let sym = program.constants[name].as_symbol();
R[dst] = globals[&sym].clone()
```
Error if global not defined.

#### `SetGlobal { name: ConstIdx, src: Reg }`
Mutate or define a top-level global variable by name.
```
let sym = program.constants[name].as_symbol();
globals.insert(sym, R[src].clone())
```

### Scope

Scoping is handled at compile time by the Resolver. The compiler tracks which registers are in use at each scope level. When a scope ends, its registers become available for reuse. There is no runtime cost for entering or leaving a scope — no opcodes are emitted for scope boundaries.

### Arithmetic (Generic)

These dispatch on runtime type. Support: Int, Float, mixed Int/Float (promotes to Float), Text concatenation (Add only), Duration, Date+Span.

#### `Add { dst: Reg, lhs: Reg, rhs: Reg }`
```
(Int(a), Int(b))       → Int(a + b)
(Float(a), Float(b))   → Float(a + b)
(Int(a), Float(b))     → Float(a as f64 + b)
(Float(a), Int(b))     → Float(a + b as f64)
(Text(a), Text(b))     → Text(a + b)
(Text(a), other)       → Text(a + other.to_display_string())
(other, Text(b))       → Text(other.to_display_string() + b)
(Duration(a), Duration(b)) → Duration(a + b)
(Date(d), Span{m,d2})  → Date(date_add_span(d, m, d2))
else                    → error
```

#### `Sub { dst: Reg, lhs: Reg, rhs: Reg }`
```
(Int(a), Int(b))       → Int(a - b)
(Float(a), Float(b))   → Float(a - b)
(Int(a), Float(b))     → Float(a as f64 - b)
(Float(a), Int(b))     → Float(a - b as f64)
(Duration(a), Duration(b)) → Duration(a - b)
(Date(d), Span{m,d2})  → Date(date_add_span(d, -m, -d2))
else                    → error
```

#### `Mul { dst: Reg, lhs: Reg, rhs: Reg }`
```
(Int(a), Int(b))       → Int(a * b)
(Float(a), Float(b))   → Float(a * b)
(Int(a), Float(b))     → Float(a as f64 * b)
(Float(a), Int(b))     → Float(a * b as f64)
else                    → error
```

#### `Div { dst: Reg, lhs: Reg, rhs: Reg }`
Same type dispatch as `Mul`. Division by zero → error.

#### `Mod { dst: Reg, lhs: Reg, rhs: Reg }`
Int % Int only. Modulo by zero → error.
```
(Int(a), Int(b))  → Int(a % b)
else              → error
```

### Arithmetic (Type-Specialized)

Emitted by the compiler when both operands are statically known to be Int. Skip all type dispatch. Directly extract i64, operate, wrap.

#### `AddInt { dst: Reg, lhs: Reg, rhs: Reg }`
```
R[dst] = Value::int(R[lhs].as_int() + R[rhs].as_int())
```

#### `SubInt { dst: Reg, lhs: Reg, rhs: Reg }`
```
R[dst] = Value::int(R[lhs].as_int() - R[rhs].as_int())
```

#### `MulInt { dst: Reg, lhs: Reg, rhs: Reg }`
```
R[dst] = Value::int(R[lhs].as_int() * R[rhs].as_int())
```

### Comparison (Generic)

All comparisons produce `Value::bool(...)`.

#### `Eq { dst: Reg, lhs: Reg, rhs: Reg }`
Structural equality. Matches `values_equal()` semantics: Int, Float (epsilon), Bool, Text, Char, Nothing, Duration, Date, Moment, Span, Time, Inductive (recursive). Collections compare by identity/length (matches existing PartialEq).

#### `NotEq { dst: Reg, lhs: Reg, rhs: Reg }`
Negation of `Eq`.

#### `Lt { dst: Reg, lhs: Reg, rhs: Reg }`
```
(Int(a), Int(b))           → a < b
(Duration(a), Duration(b)) → a < b
(Date(a), Date(b))         → a < b
(Moment(a), Moment(b))     → a < b
(Time(a), Time(b))         → a < b
(Moment(m), Time(t))       → (m % nanos_per_day) < t
(Time(t), Moment(m))       → t < (m % nanos_per_day)
else                        → error
```

#### `Gt { dst: Reg, lhs: Reg, rhs: Reg }`
Same dispatch, `>` comparison.

#### `LtEq { dst: Reg, lhs: Reg, rhs: Reg }`
Same dispatch, `<=` comparison.

#### `GtEq { dst: Reg, lhs: Reg, rhs: Reg }`
Same dispatch, `>=` comparison.

### Comparison (Type-Specialized)

Emitted when both operands are statically known Int.

#### `LtInt { dst: Reg, lhs: Reg, rhs: Reg }`
```
R[dst] = Value::bool(R[lhs].as_int() < R[rhs].as_int())
```

#### `GtInt { dst: Reg, lhs: Reg, rhs: Reg }`
#### `LtEqInt { dst: Reg, lhs: Reg, rhs: Reg }`
#### `GtEqInt { dst: Reg, lhs: Reg, rhs: Reg }`
Same pattern.

### String

#### `Concat { dst: Reg, lhs: Reg, rhs: Reg }`
Always converts both sides to display string, concatenates. Used for `BinaryOpKind::Concat` ("X combined with Y").
```
R[dst] = Value::text(R[lhs].to_display_string() + R[rhs].to_display_string())
```

### Logical

And/Or short-circuit at the compiler level using `JumpIfFalse`/`JumpIfTrue` sequences. No dedicated And/Or opcodes.

#### `Truthy { dst: Reg, src: Reg }`
Convert any value to boolean.
```
R[dst] = Value::bool(R[src].is_truthy())
```

#### `Not { dst: Reg, src: Reg }`
Logical negation.
```
R[dst] = Value::bool(!R[src].is_truthy())
```

### Control Flow

All jump targets are absolute instruction indices (not relative offsets).

#### `Jump { target: usize }`
Unconditional jump.
```
pc = target
```

#### `JumpIfTrue { cond: Reg, target: usize }`
```
if R[cond].is_truthy() { pc = target }
```

#### `JumpIfFalse { cond: Reg, target: usize }`
```
if !R[cond].is_truthy() { pc = target }
```

### Functions

#### `DefineFunc { name: ConstIdx, func_idx: FuncIdx }`
Register a user-defined function.
```
let sym = program.constants[name].as_symbol();
functions.insert(sym, FuncRef::User(func_idx))
```
The function's bytecode, parameter count, and register count are in `program.functions[func_idx]`.

#### `Call { dst: Reg, func: ConstIdx, args_start: Reg, arg_count: u8, ic: u32 }`
Call a user-defined function. The `func` constant is the function name Symbol.

Uses register windowing — the callee's parameters are the same physical registers the caller wrote the arguments to. Zero copying.

1. Look up function by symbol in `functions` table.
2. If it's a builtin, dispatch to builtin handler.
3. If it's a user function:
   a. The caller has already placed arguments in consecutive registers starting at `args_start`.
   b. Push `CallFrame { return_pc: pc, return_reg: dst, base_reg: base, func_idx }`.
   c. Set `base = args_start`. (The callee's `R[0]` IS the caller's `R[args_start]`)
   d. Extend registers to `base + func.register_count` (only non-arg registers need allocation).
   e. `pc = func.entry_pc`.

The callee sees its parameters as `R[0]..R[param_count-1]` — they're the same physical registers the caller wrote the arguments to.

**Constraint**: The register buffer (`registers: Vec<Value>`) is a single contiguous vector. `base` is just an index into it. All register access is `registers[base + reg_index]`.

`ic` field: reserved for inline cache (v1: ignored).

#### `CallBuiltin { dst: Reg, id: BuiltinId, args_start: Reg, arg_count: u8 }`
Call a builtin function by numeric ID. No call frame push — builtins execute inline.

| ID | Name | Args | Semantics |
|---|---|---|---|
| 0 | `show` | 1+ | For each arg: `emit_output(arg.to_display_string())`. Returns Nothing. |
| 1 | `length` | 1 | `R[args_start].len()` → Int. Works on List, Tuple, Set, Map, Text. |
| 2 | `format` | 1 | `R[args_start].to_display_string()` → Text. |
| 3 | `parseInt` | 1 | Parse Text to Int. Error on non-Text or parse failure. |
| 4 | `parseFloat` | 1 | Parse Text to Float. Error on non-Text or parse failure. |
| 5 | `abs` | 1 | Absolute value. Int → Int, Float → Float. |
| 6 | `min` | 2 | Min of two Ints. |
| 7 | `max` | 2 | Max of two Ints. |
| 8 | `copy` | 1 | `R[args_start].deep_clone()`. |

#### `CallExternal { dst: Reg, func: ConstIdx, args_start: Reg, arg_count: u8, ic: u32 }`
Host function hook. v1: identical to `Call`. Separated for future JIT boundary / debugger interception.

#### `Return { src: Reg }`
Return from user function with a value.
```
let frame = call_stack.pop();
let return_val = R[src].clone();
registers.truncate(frame.base_reg + caller_func.register_count);
base = frame.base_reg;
R[frame.return_reg] = return_val;
pc = frame.return_pc;
```

#### `ReturnNothing`
Return from user function with `Value::nothing()`. Same as `Return` but without a source register.

### Collections

#### `NewList { dst: Reg, start: Reg, count: u8 }`
Create a list from consecutive registers.
```
R[dst] = Value::list(vec![R[start], R[start+1], ..., R[start+count-1]])
```

#### `NewEmptyList { dst: Reg }`
```
R[dst] = Value::list(vec![])
```

#### `NewTuple { dst: Reg, start: Reg, count: u8 }`
Create an immutable tuple from consecutive registers.
```
R[dst] = Value::tuple(vec![R[start], ..., R[start+count-1]])
```

#### `NewEmptySet { dst: Reg }`
```
R[dst] = Value::set(vec![])
```

#### `NewEmptyMap { dst: Reg }`
```
R[dst] = Value::map(HashMap::new())
```

#### `NewRange { dst: Reg, start: Reg, end: Reg }`
Create a list from an inclusive integer range.
```
let s = R[start].as_int();
let e = R[end].as_int();
R[dst] = Value::list((s..=e).map(Value::int).collect())
```

#### `ListPush { list: Reg, value: Reg }`
Append value to list. Mutates in place (interior mutability via `Rc<RefCell>`).
```
R[list].list_push(R[value].clone())
```

#### `ListPop { dst: Reg, list: Reg }`
Remove and return last element. Returns Nothing if empty.
```
R[dst] = R[list].list_pop()
```

#### `Index { dst: Reg, collection: Reg, index: Reg, ic: u32 }` (+ic)
Dynamic index access. 1-indexed for ordered collections.
```
List(items) + Int(i)  → items[i - 1]  (error if out of bounds)
Tuple(items) + Int(i) → items[i - 1]
Text(s) + Int(i)      → Text(s.chars().nth(i - 1))
Map(m) + key          → m[key]  (error if key not found)
else                   → error
```

#### `SetIndex { collection: Reg, index: Reg, value: Reg }`
```
List(items) + Int(i)  → items[i - 1] = value  (error if out of bounds)
Map(m) + key          → m.insert(key, value)
else                   → error
```

#### `Slice { dst: Reg, collection: Reg, start: Reg, end: Reg }`
1-indexed, inclusive on both ends.
```
List(items) + Int(s) + Int(e) → List(items[s-1..e])
else                           → error
```

#### `Length { dst: Reg, collection: Reg }`
```
R[dst] = Value::int(R[collection].len() as i64)
```
Works on List, Tuple, Set, Map, Text.

#### `Contains { dst: Reg, collection: Reg, value: Reg }`
```
Set(items)  → any item == value
List(items) → any item == value
Map(m)      → m.contains_key(value)
Text(s)     → s.contains(value.as_text())  (also works with Char)
else        → error
```

#### `SetAdd { set: Reg, value: Reg }`
Add to set. No-op if already present (uses `values_equal` for membership check).

#### `SetRemove { set: Reg, value: Reg }`
Remove from set. No-op if not present.

#### `Union { dst: Reg, lhs: Reg, rhs: Reg }`
Set union. Preserves order. Deduplicates via `values_equal`.
```
R[dst] = R[lhs] ∪ R[rhs]
```

#### `Intersection { dst: Reg, lhs: Reg, rhs: Reg }`
Set intersection. Keeps elements from lhs that appear in rhs.
```
R[dst] = R[lhs] ∩ R[rhs]
```

#### `DeepClone { dst: Reg, src: Reg }`
Recursive deep copy. Lists, Sets, Maps, Tuples, Structs, Inductives all get fresh allocations.
```
R[dst] = R[src].deep_clone()
```

### Structs and Pattern Matching

#### `DefineStruct { name: ConstIdx, field_count: u8, fields_start: ConstIdx }`
Register a struct definition. Fields are stored as consecutive `(name_symbol, type_symbol, is_public)` triples in the constant pool starting at `fields_start`.
```
let sym = program.constants[name].as_symbol();
struct_defs.insert(sym, fields)
```

#### `NewStruct { dst: Reg, type_name: ConstIdx, field_count: u8, fields_start: Reg }` (+ic)
Create a struct instance. Field names come from `struct_defs[type_name]`, values from consecutive registers.
```
let name = interner.resolve(program.constants[type_name].as_symbol());
let fields = struct_defs[type_name];
let mut map = HashMap::new();
for i in 0..field_count {
    map.insert(fields[i].name, R[fields_start + i].clone());
}
R[dst] = Value::strukt(name, map)
```
If `field_count` is 0, fills in defaults from struct_defs (same behavior as interpreter).

#### `NewVariant { dst: Reg, enum_name: ConstIdx, variant: ConstIdx, arg_count: u8, args_start: Reg }`
Create an inductive value (enum variant).
```
R[dst] = Value::inductive(
    interner.resolve(program.constants[enum_name].as_symbol()),
    interner.resolve(program.constants[variant].as_symbol()),
    vec![R[args_start], ..., R[args_start + arg_count - 1]]
)
```

#### `GetField { dst: Reg, obj: Reg, field: StrIdx, ic: u32 }` (+ic)
Access a named field on a struct.
```
let field_name = &program.string_table[field];
R[dst] = R[obj].as_struct().fields[field_name].clone()
```
Error if `R[obj]` is not a struct or field doesn't exist.

#### `SetField { obj: Reg, field: StrIdx, value: Reg, ic: u32 }` (+ic)
Mutate a named field on a struct.
```
let field_name = &program.string_table[field];
R[obj].as_struct_mut().fields.insert(field_name, R[value].clone())
```
The compiler emits `SetField` directly on the register holding the struct. Since structs are `Box`-allocated, the register holds the box and mutation updates the contents in place.

#### `TestVariant { dst: Reg, obj: Reg, variant: ConstIdx }`
Test whether an inductive value matches a constructor name. Used in `Inspect` compilation.
```
R[dst] = Value::bool(R[obj].as_inductive().constructor == interner.resolve(program.constants[variant].as_symbol()))
```

#### `ExtractField { dst: Reg, obj: Reg, field: StrIdx, ic: u32 }` (+ic)
Extract a named field during pattern matching on a struct variant. Semantically identical to `GetField` but kept separate for IC profiling.

#### `ExtractArg { dst: Reg, obj: Reg, index: u8 }`
Extract a positional argument from an inductive value during pattern matching.
```
R[dst] = R[obj].as_inductive().args[index].clone()
```

### Iteration

Iterator state is stored in fixed slots (8 slots, supporting 8 levels of loop nesting).

```rust
enum IterState {
    Items { items: Vec<Value>, pos: usize },
    Pairs { pairs: Vec<(Value, Value)>, pos: usize },
}
```

#### `IterPrepare { slot: IterSlot, collection: Reg }`
Initialize an iterator over a collection. Materializes the collection into a `Vec` at preparation time (snapshot semantics — mutations during iteration don't affect the iteration).
```
List(items)  → IterState::Items { items: items.borrow().clone(), pos: 0 }
Set(items)   → IterState::Items { items: items.borrow().clone(), pos: 0 }
Text(s)      → IterState::Items { items: s.chars().map(|c| Value::text(c.to_string())).collect(), pos: 0 }
Map(m)       → IterState::Pairs { pairs: m.borrow().iter().map(|(k,v)| (k.clone(), v.clone())).collect(), pos: 0 }
else         → error
```

#### `IterNext { slot: IterSlot, dst: Reg, done_target: usize }`
Advance iterator. If exhausted, jump to `done_target`. Otherwise, store next value in `R[dst]`.
```
if iter_slots[slot].pos >= iter_slots[slot].items.len() {
    iter_slots[slot] = None;
    pc = done_target;
} else {
    R[dst] = iter_slots[slot].items[pos].clone();
    iter_slots[slot].pos += 1;
}
```

#### `IterNextPair { slot: IterSlot, key: Reg, val: Reg, done_target: usize }`
Advance pair iterator (Map iteration, tuple destructuring). If exhausted, jump.
```
if iter_slots[slot].pos >= iter_slots[slot].pairs.len() {
    iter_slots[slot] = None;
    pc = done_target;
} else {
    let (k, v) = &iter_slots[slot].pairs[pos];
    R[key] = k.clone();
    R[val] = v.clone();
    iter_slots[slot].pos += 1;
}
```

### Output

#### `Show { src: Reg }`
Emit a line of output. Calls the output callback if set, always appends to the output vector.
```
let line = R[src].to_display_string();
if let Some(cb) = &output_callback { (cb.borrow_mut())(line.clone()); }
output.push(line);
```

### Engine

#### `CheckFuel`
Emitted at every loop back-edge (while/repeat). Prevents infinite loops from freezing the browser tab.
```
fuel_counter -= 1;
if fuel_counter == 0 {
    fuel -= 1;
    fuel_counter = fuel_interval;
    if fuel <= 0 {
        return Err(VmError::FuelExhausted);
    }
}
```
Fuel is disabled (i64::MAX) for CLI/native. Enabled with configurable budget for WASM.

#### `Halt`
Stop execution. Emitted at the end of the top-level program.
```
return Ok(())
```

#### `Nop`
No operation. Used for alignment or as a placeholder during jump patching.

### CRDT Operations

#### `MergeCrdt { source: Reg, target: Reg }`
Merge source struct fields into target struct. Int fields are summed (GCounter semantics). Other fields are overwritten.
```
for (field, source_val) in R[source].as_struct().fields {
    let current = R[target].as_struct().fields.get(field).unwrap_or(Int(0));
    let merged = match (current, source_val) {
        (Int(a), Int(b)) → Int(a + b),
        _                → source_val,
    };
    R[target].as_struct_mut().fields.insert(field, merged);
}
```

#### `IncreaseCrdt { obj: Reg, field: StrIdx, amount: Reg }`
Increment a counter field on a struct.
```
let field_name = &program.string_table[field];
let current = R[obj].as_struct().fields.get(field_name).unwrap_or(Int(0));
R[obj].as_struct_mut().fields.insert(field_name, Int(current.as_int() + R[amount].as_int()));
```

#### `DecreaseCrdt { obj: Reg, field: StrIdx, amount: Reg }`
Decrement a counter field on a struct. Same as `IncreaseCrdt` with subtraction.

### Zone

Zones are simplified in the VM — no memory-mapped files, no capacity hints. The zone name is stored as a global for introspection; zone scoping is handled at compile time like all other scopes.

#### `ZoneBegin { name: ConstIdx, dst: Reg }`
```
R[dst] = Value::nothing()
// Zone name stored for diagnostics; scope handled by register allocation
```

#### `ZoneEnd`
```
// No-op at runtime. Zone scope boundaries handled at compile time.
```

### Security

#### `Check { subject: Reg, predicate: ConstIdx, is_capability: bool, object: Reg, source_text: StrIdx }`
Runtime security check. Evaluates policy conditions from the `PolicyRegistry`.

- `is_capability = false`: Predicate check ("user is admin"). Looks up predicate on subject's type.
- `is_capability = true`: Capability check ("user can publish document"). Looks up capability with `R[object]` as the target.

```
if !passed {
    return Err(VmError::SecurityCheckFailed(program.string_table[source_text].clone()));
}
```

### Miscellaneous

#### `Assert { cond: Reg }`
Runtime assertion. Compiled from `Stmt::RuntimeAssert`.
```
if !R[cond].is_truthy() {
    return Err(VmError::AssertionFailed);
}
```

#### `GiveTo { object: Reg, recipient: ConstIdx }`
Ownership transfer. Calls the recipient function with the object as sole argument.
```
let func_sym = program.constants[recipient].as_symbol();
call_function(func_sym, vec![R[object].clone()])
```

#### `ShowTo { object: Reg, recipient: ConstIdx }`
Immutable borrow display. If recipient resolves to `"show"`, emit output. Otherwise, call the recipient function.
```
let name = interner.resolve(program.constants[recipient].as_symbol());
if name == "show" {
    emit_output(R[object].to_display_string());
} else {
    call_function(recipient_sym, vec![R[object].clone()]);
}
```

#### `ListPushField { obj: Reg, field: StrIdx, value: Reg }`
Push a value to a list field of a struct. Used for `Stmt::Push` when the collection is a field access (`Push x to p's items`).
```
let field_name = &program.string_table[field];
R[obj].as_struct().fields[field_name].list_push(R[value].clone())
```

### Closures

Designed for upfront — these opcodes are defined in the enum but will error with "closures not yet supported" until the language feature lands. Their presence ensures the opcode numbering and VM architecture don't need restructuring later.

#### `Closure { dst: Reg, func_idx: FuncIdx, upvalue_count: u8 }`
Create a closure object. The next `upvalue_count` pseudo-ops in the bytecode stream encode where each upvalue comes from (local register or parent upvalue).
```
R[dst] = ClosureObject {
    func: program.functions[func_idx],
    upvalues: Vec with upvalue_count slots
}
```

#### `GetUpvalue { dst: Reg, slot: u8 }`
Load a captured variable from the current closure's upvalue array.
```
R[dst] = current_closure.upvalues[slot].get()
```

#### `SetUpvalue { slot: u8, src: Reg }`
Store a value into a captured variable slot.
```
current_closure.upvalues[slot].set(R[src].clone())
```

#### `CloseUpvalue { start_reg: Reg }`
When a scope ends, migrate any locals at or above `start_reg` from stack to heap if they've been captured by a closure. Converts stack references to heap references.
```
for reg in start_reg..frame_top {
    if captured[reg] {
        heap_allocate(registers[base + reg])
    }
}
```

### Tail Call Optimization

#### `CallTail { func: Reg, args_start: Reg, arg_count: u8 }`
Reuse the current CallFrame. Drop current frame's registers, copy new args to base, jump to new entry_pc. Prevents stack overflow for recursive functions.
```
let func_ref = resolve(R[func]);
// Reuse current frame — no push
copy R[args_start..args_start+arg_count] → R[base..base+arg_count]
registers.truncate(base + func.register_count)
pc = func.entry_pc
```
The compiler emits this when a `Call` is the last operation before `Return`.

### Control Flow Optimization

#### `Loop { back_offset: u16 }`
Unconditional backward jump with built-in fuel check. Equivalent to `CheckFuel` + `Jump` but as a single dispatch. Emitted at loop back-edges instead of separate `CheckFuel` + `Jump`.
```
fuel_counter -= 1;
if fuel_counter == 0 {
    fuel -= 1;
    fuel_counter = fuel_interval;
    if fuel <= 0 { return Err(VmError::FuelExhausted); }
}
pc -= back_offset as usize;
```

#### `Switch { cond: Reg, table_idx: ConstIdx }`
Jump table for Inspect (pattern matching). Constants pool entry at `table_idx` contains a `Vec<(variant_name, target_pc)>`. O(1) dispatch instead of chained `TestVariant` + `JumpIfFalse`.
```
let variant = R[cond].as_inductive().constructor;
let table = program.constants[table_idx].as_jump_table();
pc = table.lookup(variant).unwrap_or(default_pc);
```

---

## Compiler Compilation Patterns

How each AST node compiles to bytecode.

### And/Or Short-Circuit

`BinaryOpKind::And`:
```
compile(left) → R[tmp]
JumpIfFalse R[tmp] → false_label
compile(right) → R[dst]
Truthy R[dst], R[dst]
Jump → end_label
false_label: LoadFalse R[dst]
end_label:
```

`BinaryOpKind::Or`:
```
compile(left) → R[tmp]
JumpIfTrue R[tmp] → true_label
compile(right) → R[dst]
Truthy R[dst], R[dst]
Jump → end_label
true_label: LoadTrue R[dst]
end_label:
```

### While Loop

```
loop_start:
  Loop back_offset           // fuel check built in
  compile(condition) → R[cond]
  JumpIfFalse R[cond] → loop_end
  compile(body)               // locals are registers, no scope push needed
  Jump → loop_start
loop_end:
```

### For-In Loop (Repeat)

```
compile(iterable) → R[iter_val]
IterPrepare slot=N, R[iter_val]
loop_start:
  Loop back_offset
  IterNext slot=N, R[item_reg], → loop_end  // item goes directly to its register
  compile(body)
  Jump → loop_start
loop_end:
```

The loop variable is assigned a register by the Resolver. `IterNext` writes directly to that register — no `DefineVar` needed.

### If/Else

```
compile(condition) → R[cond]
JumpIfFalse R[cond] → else_label
compile(then_block)    // locals use registers allocated by resolver
Jump → end_label
else_label:
compile(else_block)
end_label:
```

Sibling branches (then/else) can reuse the same registers since they never execute together. The Resolver handles this.

### Function Definition

```
DefineFunc name=N, func_idx=F
Jump → after_body
// func entry_pc:
// Params are already in R[0]..R[param_count-1] via register windowing
compile(body)
ReturnNothing
after_body:
```

No scope push, no `DefineVar` for parameters. Register windowing means the caller's argument registers become the callee's parameter registers.

### Inspect (Pattern Matching)

```
compile(target) → R[target]
// Arm 1: variant check
TestVariant R[test], R[target], variant_1
JumpIfFalse R[test] → arm_2
ExtractField/ExtractArg R[binding_reg], R[target], field_i  // directly to resolver-assigned register
compile(arm_1_body)
Jump → end
arm_2:
// ... repeat for each arm ...
// Otherwise arm (if present):
compile(otherwise_body)
end:
```

Pattern bindings are assigned registers by the Resolver. `ExtractField`/`ExtractArg` write directly to those registers.

### Type Tracking

The compiler maintains a simple type map: `HashMap<Reg, KnownType>` where `KnownType ∈ { Int, Float, Bool, Text, Unknown }`.

- `Literal::Number(_)` → register is `Int`
- `Literal::Float(_)` → register is `Float`
- `AddInt` result → register is `Int`
- Function call result → `Unknown`
- Variable load → `Unknown` (could track through SSA, but v1 keeps it simple)

When compiling `BinaryOp::Add` with both operands `KnownType::Int`, emit `AddInt` instead of `Add`. Same for Sub, Mul, Lt, Gt, LtEq, GtEq.

### Register Allocation

Linear scan. The compiler maintains a `next_reg: u8` counter per function. Each expression result gets the next register. Registers are function-scoped (reset on function entry). 256 registers per frame.

Temporary registers for subexpressions are reused after the enclosing expression completes. The compiler tracks `high_water_mark` to determine `register_count` for the `CompiledFunction`.

---

## Data Structures

### CompiledProgram (`instruction.rs`)

```rust
#[derive(Serialize, Deserialize)]
pub struct CompiledProgram {
    pub instructions: Vec<Op>,
    pub constants: Vec<Constant>,
    pub string_table: Vec<String>,
    pub functions: Vec<CompiledFunction>,
    pub entry_pc: usize,
    pub source_map: BytecodeSourceMap,
    pub version: u32,
}
```

### CompiledFunction

```rust
#[derive(Serialize, Deserialize)]
pub struct CompiledFunction {
    pub name: ConstIdx,
    pub entry_pc: usize,
    pub param_count: u8,
    pub register_count: u16,
    pub param_names: Vec<ConstIdx>,
}
```

### Constant

```rust
#[derive(Serialize, Deserialize, Clone)]
pub enum Constant {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Char(char),
    Nothing,
    Duration(i64),
    Date(i32),
    Moment(i64),
    Span { months: i32, days: i32 },
    Time(i64),
    Symbol(Symbol),
}
```

Note: Constants are stored as their primitive values, not as `RuntimeValue`. The VM converts to `Value` on `LoadConst`. This keeps the constant pool serializable without requiring `RuntimeValue` to derive Serialize (it contains `Rc<RefCell<...>>` which can't serialize).

### CallFrame (`machine.rs`)

```rust
struct CallFrame {
    return_pc: usize,
    return_reg: Reg,
    base_reg: usize,
    func_idx: FuncIdx,
}
```

### FuncRef

```rust
enum FuncRef {
    User(FuncIdx),
    Builtin(BuiltinId),
}
```

### IterState

```rust
enum IterState {
    Items { items: Vec<Value>, pos: usize },
    Pairs { pairs: Vec<(Value, Value)>, pos: usize },
}
```

### VM (`machine.rs`)

```rust
pub struct VM<'a> {
    program: &'a CompiledProgram,
    registers: Vec<Value>,          // Single contiguous register buffer
    call_stack: Vec<CallFrame>,
    globals: HashMap<Symbol, Value>, // Top-level variables only (not locals)
    functions: HashMap<Symbol, FuncRef>,
    struct_defs: HashMap<Symbol, Vec<(Symbol, Symbol, bool)>>,
    iter_slots: [Option<IterState>; 8],
    output: Vec<String>,
    output_callback: Option<OutputCallback>,
    base: usize,                    // Current frame's register window base
    pc: usize,

    // Fuel
    fuel: i64,
    fuel_interval: u32,
    fuel_counter: u32,

    interner: &'a Interner,
    policy_registry: Option<PolicyRegistry>,
}
```

All register access is `registers[base + reg_index]`. No `Environment`, no `HashMap` lookup for local variables. Only `globals` uses a HashMap, and only for top-level variable access.

### VmError

```rust
pub enum VmError {
    Runtime(String),
    FuelExhausted,
    AssertionFailed,
    SecurityCheckFailed(String),
}
```

### BytecodeSourceMap (`sourcemap.rs`)

```rust
#[derive(Serialize, Deserialize, Default)]
pub struct BytecodeSourceMap {
    entries: Vec<(usize, Span)>,
}

impl BytecodeSourceMap {
    pub fn add(&mut self, pc: usize, span: Span);
    pub fn lookup(&self, pc: usize) -> Option<Span>;
}
```

Binary search on sorted `entries`. Reuses `Span` from `logicaffeine_base`.

---

## Dispatch Loop (`machine.rs`)

Single function. Single match. No helper method calls in the hot path. `#[repr(u8)]` on `Op` enables jump table optimization.

```rust
pub fn run(&mut self) -> Result<(), VmError> {
    loop {
        let op = &self.program.instructions[self.pc];
        self.pc += 1;
        match op {
            Op::LoadConst { dst, idx } => {
                self.registers[self.base + *dst as usize] = ...;
            }
            Op::Add { dst, lhs, rhs } => { ... }
            // ... all 87 opcodes inlined here ...
            Op::Halt => return Ok(()),
        }
    }
}
```

The entire loop body should fit in L1 instruction cache (~16KB). No function calls, no indirect dispatch, no vtables.

### Instruction Encoding

**Option A (v1): `Vec<Op>` enum** — Current design. Easy to debug, easy to serialize. Each instruction is a Rust enum variant. Size: padded to largest variant (likely 12-16 bytes per instruction).

**Option B (v2): `Vec<u8>` bytecode stream** — Cache-optimal. `Halt` = 1 byte, `LoadConst` = 3 bytes, `Add` = 4 bytes. PC indexes into byte stream. Dispatch reads opcode byte, then reads operand bytes.

**Decision: Start with `Vec<Op>` (Option A).** Correctness first. The `Op` enum is easier to debug, print, serialize, and test. Once all e2e tests pass, profile. If instruction dispatch is the bottleneck (unlikely in v1 — value allocation dominates), switch to `Vec<u8>`. The compiler emits through a `BytecodeBuilder` abstraction that hides the encoding, so the switch is localized.

---

## Unsupported AST Nodes

These AST nodes are not compiled by the bytecode compiler. Programs containing them fall through to the async tree-walker or the Rust codegen path.

| AST Node | Reason |
|---|---|
| `Stmt::Escape` | Raw Rust code. Compile-path only. |
| `Stmt::ReadFrom` | Async I/O. Tree-walker handles. |
| `Stmt::WriteFile` | Async I/O. Tree-walker handles. |
| `Stmt::Sleep` | Async. Tree-walker handles. |
| `Stmt::Mount` | Async persistence. Tree-walker handles. |
| `Stmt::Sync` | Distributed. Compile-path only. |
| `Stmt::Listen` | Networking. Compile-path only. |
| `Stmt::ConnectTo` | Networking. Compile-path only. |
| `Stmt::LetPeerAgent` | Networking. Compile-path only. |
| `Stmt::Spawn` | Agents. Compile-path only. |
| `Stmt::SendMessage` | Agents. Compile-path only. |
| `Stmt::AwaitMessage` | Agents. Compile-path only. |
| `Stmt::LaunchTask` | Concurrency. Compile-path only. |
| `Stmt::LaunchTaskWithHandle` | Concurrency. Compile-path only. |
| `Stmt::CreatePipe` | Channels. Compile-path only. |
| `Stmt::SendPipe` | Channels. Compile-path only. |
| `Stmt::ReceivePipe` | Channels. Compile-path only. |
| `Stmt::TrySendPipe` | Channels. Compile-path only. |
| `Stmt::TryReceivePipe` | Channels. Compile-path only. |
| `Stmt::StopTask` | Concurrency. Compile-path only. |
| `Stmt::Select` | Channels. Compile-path only. |
| `Stmt::AppendToSequence` | CRDT (RGA). Compile-path only. |
| `Stmt::ResolveConflict` | CRDT (MVRegister). Compile-path only. |
| `Stmt::Require` | Compilation metadata. No-op. |
| `Stmt::Theorem` | Verified at compile time. No-op. |
| `Stmt::Assert` (logic) | Logic kernel. No-op at runtime. |
| `Stmt::Trust` | Logic kernel. No-op at runtime. |
| `Expr::Escape` | Raw Rust code. |
| `Expr::ManifestOf` | Zone introspection. Stub: returns empty list. |
| `Expr::ChunkAt` | Zone introspection. Stub: returns Nothing. |

`Stmt::Concurrent` and `Stmt::Parallel` are compiled as sequential execution (same as the tree-walker does in WASM).

---

## Reserved Opcode Space

These are not implemented in v1 but the opcode numbering leaves room for them.

| Opcode | Fuses | Speedup |
|---|---|---|
| `IncLocal { reg: Reg, amount: Reg }` | `AddInt` on a local register | `R[reg] = Value::int(R[reg].as_int() + R[amount].as_int())` — single dispatch |
| `DecLocal { reg: Reg, amount: Reg }` | `SubInt` on a local register | `R[reg] = Value::int(R[reg].as_int() - R[amount].as_int())` — single dispatch |
| `CallMethod { dst: Reg, obj: Reg, method: StrIdx, args_start: Reg, arg_count: u8, ic: u32 }` | `GetField` + `Call` | Eliminates intermediate register + dispatch |
| `TestAndJump { obj: Reg, variant: ConstIdx, target: usize }` | `TestVariant` + `JumpIfFalse` | Single dispatch for pattern match arms |

---

## Inline Cache Slots

Instructions with `ic: u32` fields:

| Instruction | What IC could cache |
|---|---|
| `Call` | Resolved `FuncRef` (skip HashMap lookup) |
| `CallExternal` | Host function pointer |
| `Index` | Collection type (skip type dispatch) |
| `GetField` | Field offset in struct (skip HashMap lookup) |
| `SetField` | Field offset in struct |
| `ExtractField` | Field offset in pattern match context |
| `NewStruct` | Pre-computed field layout |

v1: all IC fields are 0 and ignored. The VM stores a parallel `Vec<u64>` for IC state, indexed by `ic`. Future phases populate these with monomorphic/polymorphic cache entries.

---

## Phased Implementation

### Phase 1: Foundation
**Files**: `vm/mod.rs`, `vm/value.rs`, `vm/instruction.rs`, `vm/sourcemap.rs`

`Value` abstraction with full API. `Op` enum with all 87 opcodes. `CompiledProgram`, `CompiledFunction`, `Constant` structs. `BytecodeSourceMap`.

**Tests**: Value round-trips (construct → extract → verify), type checks, display strings, deep_clone, collection operations. Op enum `size_of` check.

### Phase 2: Resolver + Compiler + VM Core
**Files**: `vm/resolver.rs`, `vm/compiler.rs`, `vm/machine.rs`

1. Build `resolver.rs` — walk AST, assign `Local(reg_index)` to each variable.
2. Build compiler that uses resolver output to emit `LoadConst` to assigned registers.
3. Build VM with `globals` HashMap (not `Environment`).
4. Compile and execute: `Let x be 5. Show x.`

**Opcodes activated**: LoadConst, Move, LoadNothing, LoadTrue, LoadFalse, LoadGlobal, SetGlobal, Show, Halt.

**Tests**: Compile to bytecode → verify instruction sequence (e.g., `Let x be 5. Show x.` compiles to `LoadConst R4, #5; Show R4; Halt`). Run VM → verify output matches tree-walker.

### Phase 3: Arithmetic + Comparisons + Fuel

**Opcodes activated**: Add, Sub, Mul, Div, Mod, AddInt, SubInt, MulInt, Eq, NotEq, Lt, Gt, LtEq, GtEq, LtInt, GtInt, LtEqInt, GtEqInt, Concat, Truthy, Not, CheckFuel, Loop.

Type tracking in compiler. Verify `AddInt` emitted for known-Int operands. Verify fuel exhaustion terminates.

### Phase 4: Control Flow

**Opcodes activated**: Jump, JumpIfTrue, JumpIfFalse, IterPrepare, IterNext, IterNextPair, Return, ReturnNothing.

Jump patching (forward references). CheckFuel at every loop back-edge.

**Tests**: If/else, while, for-in, break (via Jump), nested loops, early return.

### Phase 5: Functions

**Opcodes activated**: DefineFunc, Call, CallBuiltin, CallExternal, CallTail.

CallFrame stack with register windowing. Builtin dispatch table. Tail call optimization for recursive functions.

**Tests**: User functions, recursion, all 9 builtins. `fib(35)` benchmark comparison vs tree-walker. Tail-recursive function with depth > 10,000 (verifies no stack overflow).

### Phase 6: Collections

**Opcodes activated**: NewList, NewEmptyList, NewTuple, NewEmptySet, NewEmptyMap, NewRange, ListPush, ListPop, Index, SetIndex, Slice, Length, Contains, SetAdd, SetRemove, Union, Intersection, DeepClone.

### Phase 7: Structs + Pattern Matching

**Opcodes activated**: DefineStruct, NewStruct, NewVariant, GetField, SetField, TestVariant, ExtractField, ExtractArg, ListPushField, Switch.

### Phase 8: Integration

Wire VM into `interpret_for_ui_sync`. Activate remaining opcodes: Assert, GiveTo, ShowTo, MergeCrdt, IncreaseCrdt, DecreaseCrdt, ZoneBegin, ZoneEnd, Check, Nop. Closure opcodes (Closure, GetUpvalue, SetUpvalue, CloseUpvalue) are defined but error with "closures not yet supported". Add Serialize/Deserialize to CompiledProgram.

**Gate**: ALL existing e2e tests produce identical output.

### Phase 9: Benchmark + Profile

Full benchmark suite. Profile hot paths. Targeted optimizations.

---

## Files Modified

| File | Change |
|---|---|
| `crates/logicaffeine_compile/src/lib.rs` | `pub mod vm;` |
| `crates/logicaffeine_compile/src/ui_bridge.rs` | Wire VM into `interpret_for_ui_sync` sync path |

## Files Created

| File | ~Lines | Purpose |
|---|---|---|
| `vm/mod.rs` | 50 | Public API |
| `vm/value.rs` | 400 | Value newtype |
| `vm/instruction.rs` | 400 | Op enum, CompiledProgram, Constant |
| `vm/resolver.rs` | 150 | Scope resolution: Symbol → Local(Reg) / Global / Upvalue(slot) |
| `vm/compiler.rs` | 1,200 | AST → bytecode |
| `vm/machine.rs` | 1,100 | Dispatch loop, register windowing |
| `vm/sourcemap.rs` | 50 | Bytecode → source span |

~3,350 lines total.

---

## Invariants

1. All value access goes through `Value` methods — never match on `RuntimeValue` in VM code.
2. All cacheable instructions carry `ic: u32` — even if unused in v1.
3. The dispatch loop is one function, one match — no helper calls in the hot path.
4. Call frames are explicit heap `Vec<CallFrame>` — zero Rust stack recursion.
5. The async tree-walker stays — VM is sync-only, async path unchanged.
6. Fuel check at every loop back-edge (via `Loop` opcode).
7. `CompiledProgram` is `Serialize + Deserialize`.
8. `BytecodeSourceMap` tracks every instruction.
9. No HashMap lookup for local variables — locals are registers, resolved at compile time by the Resolver. Only `globals` uses name-based lookup.
10. Register windowing for function calls — zero argument copying. The callee's `R[0]` is the caller's `R[args_start]`.
