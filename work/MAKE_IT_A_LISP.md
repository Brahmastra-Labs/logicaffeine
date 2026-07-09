# MAKE_IT_A_LISP.md — The Path to Homoiconicity

LogicAffeine is 85% of the way to being a Lisp. Not a Lisp that looks like
Lisp — no parentheses, no `car`/`cdr`, no linked-list fetishism — but a
language with the *essential property* that makes Lisp what it is: **code is
data, and data is code**.

This document maps the gap between what we have and what we need.

---

## What Makes a Lisp

Strip away the syntax. Strip away the history. A Lisp is a language where:

1. **Programs are values.** Code can be constructed, inspected, and
   transformed as ordinary data at runtime.
2. **`quote` freezes code into data.** Any expression can be suspended into
   its syntactic representation without evaluating it.
3. **`eval` thaws data into code.** Any well-formed data structure can be
   executed as a program.
4. **These two operations are inverses.** `eval(quote(e)) = e` for all `e`.

Everything else — macros, `defmacro`, `syntax-rules`, hygienic expansion,
reader macros, quasi-quotation — is a consequence of these four properties.
The quote/eval duality is the atom. Everything else is chemistry.

---

## What LogicAffeine Already Has

### The Self-Interpreter (Sprint 4)

LogicAffeine has a complete self-interpreter: a LOGOS program that interprets
LOGOS programs. It lives in the Futamura test suite (`phase_futamura.rs`) and
consists of:

**CExpr** — code-as-data for expressions:
```
A CInt with value Int.
A CBool with value Bool.
A CText with value Text.
A CVar with name Text.
A CBinOp with op Text and left CExpr and right CExpr.
A CNot with inner CExpr.
A CCall with name Text and args Seq of CExpr.
A CIndex with coll CExpr and idx CExpr.
A CLen with target CExpr.
A CMapGet with target CExpr and key CExpr.
A CNewSeq.
A CNewVariant with tag Text and fnames Seq of Text and fvals Seq of CExpr.
```

**CStmt** — code-as-data for statements:
```
A CLet with name Text and expr CExpr.
A CSet with name Text and expr CExpr.
A CIf with cond CExpr and thenBlock Seq of CStmt and elseBlock Seq of CStmt.
A CWhile with cond CExpr and body Seq of CStmt.
A CReturn with expr CExpr.
A CShow with expr CExpr.
A CCallS with name Text and args Seq of CExpr.
A CPush with expr CExpr and target Text.
A CSetIdx with target Text and idx CExpr and val CExpr.
A CMapSet with target Text and key CExpr and val CExpr.
A CPop with target Text.
```

**CVal** — runtime values in the meta-level:
```
A VInt with value Int.
A VBool with value Bool.
A VText with value Text.
A VSeq with items Seq of CVal.
A VMap with entries Map of Text to CVal.
A VError with msg Text.
A VNothing.
```

**coreEval** — the heart of the self-interpreter. It takes a `CExpr`, an
environment (`Map of Text to CVal`), and a function table (`Map of Text to
CFunc`), and produces a `CVal`. This is `eval` — it already exists.

**coreExecBlock** — executes a `Seq of CStmt` in an environment. This is
the statement-level evaluator.

### The Encoder (`compile.rs`)

`encode_program_source(source) -> Result<String, ParseError>` takes any LOGOS
source and produces LOGOS code that constructs the equivalent `CProgram` data
structure. This is **reification** — it turns code into data. It already exists.

### Binding-Time Analysis (`optimize/bta.rs`)

The BTA classifies every variable as **Static** (value known at compile time)
or **Dynamic** (depends on runtime input). It's polyvariant: the same function
at different call sites with different argument patterns produces different
divisions. Types: `BindingTime::Static(Literal)` / `BindingTime::Dynamic`.
Entry point: `BtaEnv::analyze_source(source)`.

### Function Specialization (`optimize/partial_eval.rs`)

The partial evaluator specializes functions with mixed static/dynamic
arguments. Given `f(3, x)` where 3 is static and `x` is dynamic, it
produces `f_s0_3(x)` with the static argument baked in. It cascades
(specialized functions can trigger further specializations), uses
homeomorphic embedding for termination, and caps at 8 variants per function.
Entry point: `specialize_stmts(stmts, expr_arena, stmt_arena, interner)`.

### The Supercompiler (`optimize/supercompile.rs`)

Online partial evaluation via driving (symbolic execution), folding
(memoization), and generalization (widening). Handles pure integer/boolean
code. Includes `embeds()` for homeomorphic embedding and `msg()` for most
specific generalization. Entry point:
`supercompile_stmts(stmts, expr_arena, stmt_arena, interner)`.

### CTFE (`optimize/ctfe.rs`)

Compile-time function evaluation. Evaluates pure function calls with
all-literal arguments, replacing the call with its result. Step-limited
(10,000 steps, depth 16).

### All Three Futamura Projections (103 tests, all green)

- **Projection 1**: `pe(interpreter, program) = compiled_program`
  `pe_source_text()` provides the partial evaluator written in LOGOS itself.
  Specializing the self-interpreter with respect to a program produces a
  compiled version that runs without the interpreter dispatch.

- **Projection 2**: `pe(pe, interpreter) = compiler`
  `projection2_source()` specializes the PE with respect to the
  self-interpreter, producing a compiler. Feed it any program → get compiled
  output.

- **Projection 3**: `pe(pe, pe) = compiler_generator`
  `projection3_source()` specializes the PE with respect to itself,
  producing a compiler generator. Feed it any interpreter → get a compiler
  for that language. Tested with Core interpreter AND an RPN calculator.

### Closures, Enums, Pattern Matching

LogicAffeine already has first-class closures, algebraic data types via
`Inspect`/`When`, and pattern matching with destructuring. These are the
building blocks of macro systems.

---

## What's Missing: The Four Pieces

All the machinery exists *inside the compiler*. The gap is surfacing it to
user code.

### 1. Quote Syntax

**The problem:** There's no way for a user to write `Quote (x + 3)` and get
back a `CExpr` value. Today, constructing code-as-data requires manually
building CExpr trees:

```
Let left be a new CVar with name "x".
Let right be a new CInt with value 3.
Let expr be a new CBinOp with op "+" and left left and right right.
```

**The solution:** A `Quote` expression in the parser that desugars to CExpr
construction.

```
Let expr be Quote (x + 3).
```

Desugars to the manual construction above. Variables in scope become `CVar`
nodes. Literals become `CInt`/`CBool`/`CText` nodes. Operators become
`CBinOp` nodes. Calls become `CCall` nodes.

**Unquote** (`$`) allows splicing runtime values back into quoted code:

```
Let n be 42.
Let expr be Quote (x + $n).
```

Here `$n` evaluates `n` at construction time and embeds the result as
`CInt(42)`, while `x` stays as `CVar("x")`.

**Files to modify:**
- `lexer.rs` — new `Quote` / `Unquote` tokens
- `parser/mod.rs` — `parse_quote_expr()` that walks the quoted form and
  emits CExpr construction statements
- `ast/stmt.rs` — `Expr::Quote { body }` and `Expr::Unquote { expr }` variants

### 2. Eval Primitive

**The problem:** The self-interpreter (`coreEval`/`coreExecBlock`) exists
but is only available as a user-defined function in the Futamura test suite.
There's no built-in way to evaluate a `CExpr` or execute a `Seq of CStmt`.

**The solution:** A built-in `Eval` statement/expression that dispatches
to `coreEval`/`coreExecBlock`:

```
Let result be Eval expr with env.
```

Or more simply:

```
Let result be Eval (Quote (2 + 3)).
```

This needs a runtime representation of environments. The CVal type system
already handles this — `Map of Text to CVal` is the environment.

**Two modes:**
1. **Expression eval**: `Eval expr` → evaluates a `CExpr`, returns a `CVal`
2. **Block eval**: `Run stmts` → executes a `Seq of CStmt` for side effects

**Files to modify:**
- `parser/mod.rs` — parse `Eval` as an expression, `Run` as a statement
- `ast/stmt.rs` — `Expr::Eval { expr, env }` and `Stmt::Run { stmts, env }`
- `codegen/expr.rs` — emit calls to the built-in interpreter
- `interpreter.rs` — direct dispatch to `coreEval`/`coreExecBlock`

### 3. RuntimeValue::Code

**The problem:** The interpreter's `RuntimeValue` enum doesn't have a
variant for code-as-data. CExpr/CStmt/CVal are user-defined enum types
that go through the generic enum machinery. This works but is slow and
loses type safety.

**The solution:** Add `RuntimeValue::Code(CExpr)` as a first-class runtime
value. This lets the interpreter handle quoted expressions natively without
boxing through the generic enum path.

```rust
enum RuntimeValue {
    // ... existing variants ...
    Code(Box<CodeValue>),
}

enum CodeValue {
    Expr(CExprData),
    Stmt(CStmtData),
    Program(CProgramData),
}
```

This is an optimization, not a requirement. The system works without it
(CExpr as a user-defined enum is fine), but native code values enable:
- Pattern matching on code structure in `Inspect`
- Efficient code transformation without Map overhead
- Type-safe quote/eval round-tripping

**Files to modify:**
- `interpreter.rs` — new `RuntimeValue::Code` variant
- `codegen/expr.rs` — code literal emission
- `analysis/unify.rs` — type unification for Code types

### 4. The Reification Bridge

**The problem:** `encode_program_source()` exists in Rust but isn't
accessible from LOGOS user code. A user can't take a string of LOGOS source
and get back a `CProgram` at runtime.

**The solution:** A built-in function that bridges the compiler's encoder
to user code:

```
Let program be Parse "Let x be 5. Show x + 3.".
```

This calls `encode_program_source()` under the hood and returns a `CProgram`
value. Combined with `Eval`, this completes the circle:

```
Let code be Parse "Show 2 + 3.".
Run code.
```

Output: `5`

**Files to modify:**
- `parser/mod.rs` — parse `Parse` as an expression returning CProgram
- `codegen/expr.rs` — emit call to runtime parsing function
- `interpreter.rs` — call `encode_program_source` + `coreExecBlock`
- For compiled output: embed a minimal parser/encoder in the runtime

---

## Sprint Plan

### Sprint 1: Quote and Unquote (Syntax → Data)

**Goal:** `Quote (expr)` produces a `CExpr` value. `$var` inside quotes
splices runtime values.

**RED tests:**
```
// Quote literal → CInt
Let q be Quote 42.
Inspect q:
    When CInt (n):
        Show n.
// Expected: "42"

// Quote binary op → CBinOp
Let q be Quote (2 + 3).
Inspect q:
    When CBinOp (op, left, right):
        Show op.
// Expected: "+"

// Unquote splices values
Let n be 10.
Let q be Quote (x + $n).
Inspect q:
    When CBinOp (op, left, right):
        Inspect right:
            When CInt (v):
                Show v.
// Expected: "10"
```

**Implementation:**
1. Add `Quote`/`Unquote` tokens to lexer
2. Add `Expr::Quote`/`Expr::Unquote` to AST
3. Parser: `parse_quote_expr()` walks the quoted form, emitting CExpr
   construction. Variables → `CVar`, literals → `CInt`/`CBool`/`CText`,
   operators → `CBinOp`, `$expr` → evaluate and wrap
4. Codegen: Quote expressions compile to CExpr constructor calls
5. Interpreter: Quote expressions evaluate to CExpr enum values

**Depends on:** CExpr/CStmt/CVal types being declared as built-in enums
or auto-imported when Quote is used.

### Sprint 2: Eval Primitive (Data → Execution)

**Goal:** `Eval expr` evaluates a CExpr and returns a CVal. `Run stmts`
executes a block of CStmts.

**RED tests:**
```
// Eval a quoted literal
Let result be Eval (Quote 42).
Inspect result:
    When VInt (n):
        Show n.
// Expected: "42"

// Eval a quoted expression
Let result be Eval (Quote (2 + 3)).
Inspect result:
    When VInt (n):
        Show n.
// Expected: "5"

// Run a statement block
Let showStmt be a new CShow with expr (Quote 99).
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Run stmts.
// Expected: "99"
```

**Implementation:**
1. Parser: `Eval` as expression (returns CVal), `Run` as statement
2. Codegen: Emit calls to embedded `coreEval`/`coreExecBlock`
3. Interpreter: Direct dispatch to evaluation logic
4. The self-interpreter functions become part of the standard library,
   auto-included when `Eval`/`Run` are used

### Sprint 3: Quote Blocks (Statement Quotation)

**Goal:** Quote entire statement blocks, not just expressions.

**RED tests:**
```
// Quote a block → Seq of CStmt
Let block be QuoteBlock:
    Let x be 5.
    Show x + 3.

Run block.
// Expected: "8"

// Manipulate quoted blocks
Let block be QuoteBlock:
    Show 1.
    Show 2.

Show length of block.
// Expected: "2"
```

**Implementation:**
1. Parser: `QuoteBlock:` starts a quoted block, each statement becomes
   a CStmt value
2. The block is a `Seq of CStmt` — ordinary collection, can be
   pushed to, indexed, iterated
3. Statement quotation follows the same pattern as expression quotation:
   each statement form maps to its CStmt constructor

### Sprint 4: Parse Primitive (String → Data)

**Goal:** `Parse "source"` returns a CProgram. Completes the reification
bridge.

**RED tests:**
```
// Parse and run
Let code be Parse "Show 2 + 3.".
Inspect code:
    When CProg (funcs, main):
        Let env be a new Map of Text to CVal.
        Let funcMap be a new Map of Text to CFunc.
        Let result be coreExecBlock(main, env, funcMap).
// Expected: "5"

// Parse, transform, run
Let code be Parse "Let x be 10. Show x.".
Inspect code:
    When CProg (funcs, main):
        // Replace the literal 10 with 20
        Let env be a new Map of Text to CVal.
        Let funcMap be a new Map of Text to CFunc.
        Let result be coreExecBlock(main, env, funcMap).
// Expected: "10"
```

**Implementation:**
1. Built-in `Parse` expression in parser
2. Runtime: calls `encode_program_source()` (already exists in Rust)
3. Returns a CProgram value that can be inspected, transformed, or executed

### Sprint 5: Macros via Quote/Eval

**Goal:** User-defined macros as functions that take and return code.

With Quote, Eval, and code-as-data, macros are just functions:

```
## To unless (condition: CExpr) and (body: Seq of CStmt) -> Seq of CStmt:
    Let negated be a new CNot with inner condition.
    Let ifStmt be a new CIf with cond negated and thenBlock body and elseBlock (a new Seq of CStmt).
    Let result be a new Seq of CStmt.
    Push ifStmt to result.
    Return result.

## Main
Let cond be Quote (x is greater than 10).
Let body be QuoteBlock:
    Show "x is small".
Let expanded be unless(cond, body).
```

No special macro system needed. Functions that operate on CExpr/CStmt *are*
macros. The BTA can classify macro arguments as static and the PE can
specialize them away at compile time.

### Sprint 6: Compile-Time Macros (BTA-Guided Expansion)

**Goal:** Macros that expand at compile time, not runtime.

The BTA already knows which arguments are static. A macro whose arguments are
all static can be expanded at compile time by the partial evaluator:

```
## To repeat (n: Int) and (body: Seq of CStmt) -> Seq of CStmt:
    Let result be a new Seq of CStmt.
    Let mutable i be 0.
    While i is less than n:
        Repeat for stmt in body:
            Push stmt to result.
        Set i to i + 1.
    Return result.
```

When called as `repeat(3, QuoteBlock: Show "hello".)`, the PE sees that `n=3`
is static and `body` is static, fully evaluates the function at compile time,
and emits three `Show` statements inline. No runtime overhead. No macro
expander. Just partial evaluation.

---

## Why This Is Better Than Traditional Lisp

### 1. Typed Metaprogramming

Traditional Lisps have untyped code-as-data. A quoted S-expression is just a
list — the macro can return any garbage and it won't be caught until runtime.
LogicAffeine's CExpr/CStmt types are algebraic data types with exhaustive
pattern matching. A macro that returns a CExpr is *guaranteed* to produce
well-formed code because the type system enforces it.

```
// This won't compile — CExpr doesn't have a "garbage" variant
Let q be a new CBinOp with op "+" and left "not an expr" and right 42.
//                                       ^^^^^^^^^^^^^^^^
//                                       Type error: expected CExpr, got Text
```

### 2. BTA-Guided Macro Expansion

In Lisp, macros are expanded in a separate phase with its own evaluation
semantics. In LogicAffeine, "macro expansion" is just partial evaluation.
The BTA tells you which macro arguments are known at compile time. The PE
specializes the macro body with those values. If all arguments are static,
the macro disappears entirely — zero runtime cost, proven by the same
framework that handles all other optimizations.

### 3. Proven-Correct Specialization

Futamura Projection 1 proves that specializing an interpreter with respect
to a program produces a correct compiled version. This means:

```
Eval (Quote (2 + 3))
```

can be optimized to just `5` by the PE, with a *proof* that the optimization
is semantics-preserving. No ad-hoc macro expansion. No phase-ordering bugs.
The same PE that achieves P1-P3 handles code generation from quoted
expressions.

### 4. No Phase Distinction

Lisp has a sharp distinction between compile time and runtime: macros run at
compile time, functions run at runtime, and `eval-when` mediates between
them. LogicAffeine has *one* evaluation mechanism (the PE/supercompiler) that
smoothly handles the entire spectrum from "fully static" to "fully dynamic"
based on the BTA. A function can be a macro at one call site and a runtime
function at another, depending on what's known.

### 5. Self-Application

LogicAffeine's PE is self-applicable — it can specialize itself. This means
the macro system can be used to optimize the macro system. Projection 3
(compiler generator) is the ultimate expression of this: feed it any
interpreter and it produces a compiler. No Lisp has ever achieved Futamura
Projection 3 in a production setting.

---

## Key Files Reference

| File | What It Contains |
|------|-----------------|
| `optimize/bta.rs` | Binding-Time Analysis — `BindingTime`, `Division`, `BtaEnv` |
| `optimize/partial_eval.rs` | Function specialization — `specialize_stmts()`, `SpecRegistry` |
| `optimize/supercompile.rs` | Supercompiler — `supercompile_stmts()`, `embeds()`, `msg()` |
| `optimize/ctfe.rs` | Compile-Time Function Evaluation — `ctfe_stmts()` |
| `compile.rs:2646` | `pe_source_text()` — PE written in LOGOS |
| `compile.rs:2701` | `quote_pe_source()` — encodes PE as CProgram |
| `compile.rs:2721` | `projection2_source()` — P2 compiler generation |
| `compile.rs:2748` | `projection3_source()` — P3 compiler generator |
| `compile.rs:1249` | `encode_program_source()` — reification bridge |
| `compile.rs:139` | `interpret_program()` — runtime interpreter |
| `tests/phase_futamura.rs:8-50` | CExpr/CStmt/CVal/CFunc/CProgram type definitions |
| `tests/phase_futamura.rs:52-498` | Self-interpreter (coreEval, coreExecBlock, applyBinOp) |
| `ast/stmt.rs` | AST types — `Expr`, `Stmt`, `Literal` |
| `parser/mod.rs` | Parser — where Quote/Eval syntax would be added |
| `lexer.rs` | Lexer — where Quote/Unquote tokens would be added |
| `codegen/expr.rs` | Expression codegen — where Quote/Eval emission would go |

---

## Summary

LogicAffeine has `eval` (coreEval), it has reification (encode_program_source),
it has a self-interpreter, it has BTA, it has a self-applicable partial
evaluator, and it has all three Futamura projections. What it's missing is
the *syntax* to make these accessible to user code: `Quote`, `Eval`, and
`Parse`. Three keywords. Six sprints. The infrastructure is built. The
surface just needs to be exposed.

The result isn't a Lisp clone. It's a Lisp that's *better than Lisp* — typed
code-as-data, BTA-guided macro expansion, proven-correct specialization,
and self-applicable metaprogramming, all expressed in English-like syntax
instead of parentheses.
