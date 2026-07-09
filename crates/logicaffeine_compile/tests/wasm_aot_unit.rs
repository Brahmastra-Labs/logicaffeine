//! Direct AOT WebAssembly backend — per-feature units (P0: whole-program scalar compute).
//!
//! `compile_to_wasm(src)` emits a SELF-CONTAINED `.wasm` module (no rustc/cargo): an exported
//! `main` plus the user's functions, importing only the host's `env.print_*` output sinks.
//! Each test compiles a program, runs the module through the conformant `wasmi` interpreter
//! (instantiation validates the bytes), and asserts its captured output equals the tree-walker
//! oracle byte-for-byte. The host `print_*` functions reproduce the tree-walker's
//! `to_display_string` formatting for each scalar kind, so a formatting divergence is caught.
//!
//! Native + `wasm-jit` only (the test uses `wasmi` to run the emitted module; the emitter
//! itself is feature-independent).

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome};

/// Instantiate the emitted module through `wasmi`, supplying `env.print_i64`/`print_f64`/
/// `print_bool` host sinks that capture each `Show` as one output line, then call the exported
/// `main`. Returns the captured lines joined by '\n' — directly comparable to a [`RunOutcome`]'s
/// `output`. The formatting in each sink mirrors the tree-walker's `to_display_string`.
fn run_aot(module: &[u8]) -> String {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let out: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let mut store = wasmi::Store::new(&engine, out.clone());
    let mut linker = wasmi::Linker::<Rc<RefCell<Vec<String>>>>::new(&engine);
    linker
        .func_wrap("env", "print_i64", |caller: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
            caller.data().borrow_mut().push(v.to_string());
        })
        .unwrap();
    linker
        .func_wrap("env", "print_bool", |caller: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i32| {
            caller.data().borrow_mut().push(if v != 0 { "true".into() } else { "false".into() });
        })
        .unwrap();
    linker
        .func_wrap("env", "print_nothing", |caller: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>| {
            caller.data().borrow_mut().push("nothing".into());
        })
        .unwrap();
    linker
        .func_wrap("env", "print_f64", |caller: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: wasmi::core::F64| {
            caller.data().borrow_mut().push(logicaffeine_compile::compile::display_float_like_logos(f64::from(v)));
        })
        .unwrap();
    let instance = linker.instantiate(&mut store, &m).unwrap().start(&mut store).unwrap();
    let main = instance.get_typed_func::<(), ()>(&store, "main").unwrap();
    main.call(&mut store, ()).expect("main runs without trapping");
    let lines = out.borrow().clone();
    lines.join("\n")
}

/// Compile `src` to wasm, run it, and assert byte-identical to the tree-walker oracle (which
/// must itself succeed — no error). Returns nothing; panics with a diff on mismatch.
fn assert_aot_matches_treewalker(src: &str) {
    let oracle = tw_outcome(src);
    assert_eq!(oracle.error, None, "tree-walker oracle errored on:\n{src}");
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
    let got = run_aot(&module);
    assert_eq!(got, oracle.output, "AOT wasm output disagrees with the tree-walker for:\n{src}");
}

#[test]
fn aot_shows_an_integer_literal() {
    assert_aot_matches_treewalker("## Main\n    Show 7.\n");
}

#[test]
fn aot_shows_an_integer_expression() {
    assert_aot_matches_treewalker("## Main\n    Let n be 6 * 7.\n    Show n.\n");
}

#[test]
fn aot_shows_multiple_lines() {
    assert_aot_matches_treewalker("## Main\n    Show 1.\n    Show 2.\n    Show 3.\n");
}

/// NESTED struct-parameter field access — `b's corner's x`, one level deeper than a direct field.
/// The seeded parameter layout carries each struct-typed field's TYPE NAME, so the first `GetField`
/// (`b's corner`) re-seeds its result's own layout (the `Point`'s fields) and the second (`'s x`)
/// resolves cross-region. Without the name in the layout this rejected; now it compiles and agrees.
#[test]
fn aot_nested_struct_param_field_access() {
    assert_aot_matches_treewalker(
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## A Box has:\n    A corner: Point.\n    A tag: Int.\n\n\
         ## To cornerx (b: Box) -> Int:\n    Return b's corner's x.\n\
         ## Main\n    Let p be a new Point with x 3 and y 4.\n\
         Let bx be a new Box with corner p and tag 9.\n    Show cornerx(bx).\n",
    );
}

/// Nested field access on a CALL RESULT — `b's corner's x` where `b` is a struct RETURNED by a
/// function (not a parameter). The returned struct's struct-typed field is named from the callee's
/// `struct_name_of` (the field value's `NewStruct` type), so the caller re-seeds and resolves the
/// deeper field — symmetric with the parameter path. The inner `Point` is built inline in the
/// `Return` so the returned struct is fully self-contained in one expression.
#[test]
fn aot_nested_call_result_field_access() {
    assert_aot_matches_treewalker(
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## A Box has:\n    A corner: Point.\n    A tag: Int.\n\n\
         ## To makebox () -> Box:\n    \
         Return a new Box with corner (a new Point with x 7 and y 8) and tag 5.\n\
         ## Main\n    Let b be makebox().\n    Show b's corner's x.\n",
    );
}

/// Two levels of nesting — `g's mid's inner's v` — proving the re-seed is recursive, not a
/// hard-coded single level: each `GetField` on a struct-typed field re-seeds the next layer.
#[test]
fn aot_double_nested_struct_param_field_access() {
    assert_aot_matches_treewalker(
        "## A Inner has:\n    A v: Int.\n\n\
         ## A Mid has:\n    A inner: Inner.\n\n\
         ## A Grp has:\n    A mid: Mid.\n\n\
         ## To deep (g: Grp) -> Int:\n    Return g's mid's inner's v.\n\
         ## Main\n    Let i be a new Inner with v 42.\n\
         Let m be a new Mid with inner i.\n\
         Let g be a new Grp with mid m.\n    Show deep(g).\n",
    );
}

/// MAP parameter — `f(m: Map of Int to Int)`. The value kind for `item k of m` is carried by
/// `BoundaryType::Map`'s value element and seeded as `index_value_kind`, so a parameter map's read
/// resolves cross-region with no `SetIndex` in the body. Mutation through the (reference-semantic)
/// handle is visible to the caller — byte-identical to the tree-walker.
#[test]
fn aot_map_param_read_and_mutate() {
    assert_aot_matches_treewalker(
        "## To readv (m: Map of Int to Int) -> Int:\n    Return item 1 of m + item 2 of m.\n\
         ## To bump (m: Map of Int to Int):\n    Set item 1 of m to 100.\n\
         ## Main\n    Let mutable m be a new Map of Int to Int.\n\
         Set item 1 of m to 10.\n    Set item 2 of m to 20.\n\
         Show readv(m).\n    bump(m).\n    Show item 1 of m.\n",
    );
}

/// TUPLE parameter — `f(p: Pair of Int and Int)`. A homogeneous tuple lays out identically to a
/// `Seq`, so the parameter resolves through the seq path (`Pair`→`Seq(Int)`) and `item N of p`
/// indexes the buffer — no tuple-specific seeding needed.
#[test]
fn aot_tuple_param_index() {
    assert_aot_matches_treewalker(
        "## To addpair (p: Pair of Int and Int) -> Int:\n    Return item 1 of p + item 2 of p.\n\
         ## Main\n    Let p be (3, 4).\n    Show addpair(p).\n",
    );
}

/// PAYLOAD-ENUM parameter — `f(s: Shape)` with `When V (binds)`. The bound payload kinds come from
/// the bytecode's `enum_types` (seeded as `ParamShape::Enum`); `struct_layout` pairs each `BindArm`
/// with its `TestArm` variant to resolve `field_kinds[index]`. Single- and multi-field variants,
/// plus a nullary variant mixed in.
#[test]
fn aot_enum_payload_param() {
    assert_aot_matches_treewalker(
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## To area (s: Shape) -> Int:\n    Inspect s:\n        When Circle (rad): Return rad.\n        \
         When Rectangle (w, h): Return w * h.\n    Return 0.\n\
         ## Main\n    Let c be a new Circle with radius 7.\n\
         Let r be a new Rectangle with width 3 and height 4.\n    Show area(c).\n    Show area(r).\n",
    );
}

/// STRUCT-WITH-COMPOSITE-FIELD parameter — a struct carrying BOTH a `Map` field and an enum field.
/// `GetField` on each re-seeds the result's resolution (`FieldNested::Map` → value kind,
/// `FieldNested::Enum` → variant layout), so `item k of b's m` and `Inspect b's st` both resolve
/// cross-region — the field-composed analog of direct map/enum parameters.
#[test]
fn aot_struct_with_map_and_enum_field_param() {
    assert_aot_matches_treewalker(
        "## A St is one of:\n    A Open.\n    A Closed with code Int.\n\n\
         ## A Box has:\n    A id: Int.\n    A m: Map of Int to Int.\n    A st: St.\n\n\
         ## To rd (b: Box) -> Int:\n    Inspect b's st:\n        When Open: Return item 1 of b's m.\n        \
         When Closed (code): Return code.\n    Return 0.\n\
         ## Main\n    Let mutable mm be a new Map of Int to Int.\n    Set item 1 of mm to 99.\n\
         Let o be a new Open.\n    Let b1 be a new Box with id 1 and m mm and st o.\n\
         Let c be a new Closed with code 7.\n    Let b2 be a new Box with id 2 and m mm and st c.\n\
         Show rd(b1).\n    Show rd(b2).\n",
    );
}

/// STRUCT-WITH-TUPLE-FIELD parameter — a homogeneous tuple field maps to `Seq`, so `item N of h's pr`
/// resolves through the seq element-kind path (no `FieldNested` re-seed). Pairs with another composite
/// field (a map) in the same struct.
#[test]
fn aot_struct_with_tuple_field_param() {
    assert_aot_matches_treewalker(
        "## A Box has:\n    A pr: Pair of Int and Int.\n    A m: Map of Int to Int.\n\n\
         ## To go (b: Box) -> Int:\n    Return item 1 of b's pr + item 2 of b's pr + item 1 of b's m.\n\
         ## Main\n    Let mutable mm be a new Map of Int to Int.\n    Set item 1 of mm to 100.\n\
         Let p be (3, 4).\n    Let b be a new Box with pr p and m mm.\n    Show go(b).\n",
    );
}

/// RETURN-TYPE composites used INLINE by the caller. A function returning a `Map`/enum carries its
/// declared `return_type` in the bytecode, seeded at the `Call` (the return-side analog of the
/// parameter shapes), so `item k of f()` (map) and `Inspect f()` (enum) resolve cross-region.
#[test]
fn aot_map_return_indexed_inline() {
    assert_aot_matches_treewalker(
        "## To build () -> Map of Int to Int:\n    Let mutable m be a new Map of Int to Int.\n    \
         Set item 1 of m to 42.\n    Set item 2 of m to 8.\n    Return m.\n\
         ## Main\n    Let m be build().\n    Show item 1 of m + item 2 of m.\n",
    );
}

#[test]
fn aot_enum_return_inspected_inline() {
    assert_aot_matches_treewalker(
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## To mk () -> Shape:\n    Return a new Rectangle with width 3 and height 4.\n\
         ## Main\n    Let s be mk().\n    Inspect s:\n        When Circle (r): Show r.\n        \
         When Rectangle (w, h): Show w * h.\n",
    );
}

/// CLOSURE OVER A COMPOSITE — a closure capturing a `Seq`. The captured value is a heap handle (i32),
/// so the capture slot is stored/loaded/typed at the captured global's kind (`capture_valtype`), and
/// the body's `item i of xs`/`length of xs` resolve through the self-describing `SeqInt` kind.
#[test]
fn aot_closure_captures_a_seq() {
    assert_aot_matches_treewalker(
        "## Main\nLet xs be [10, 20, 30].\n\
         Let pick be (i: Int) -> item i of xs.\n\
         Let off be 100.\n\
         Let both be (i: Int) -> item i of xs + off.\n\
         Show pick(2).\n    Show both(3).\n",
    );
}

/// CROSS-SCOPE CAPTURE — a closure built inside a FUNCTION over a function-local/param composite (no
/// global index). The capture kind/shape is read from the enclosing region's plan at the `MakeClosure`
/// site (the planner plans non-closure regions first), so a closure over a function-param `Seq` and a
/// local-built struct resolve like global captures.
#[test]
fn aot_closure_captures_function_local() {
    assert_aot_matches_treewalker(
        "## A Pt has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To run (xs: Seq of Int) -> Int:\n    Let p be a new Pt with x 100 and y 5.\n    \
         Let pick be (i: Int) -> item i of xs.\n    Let getx be (n: Int) -> n + p's x.\n    \
         Return pick(1) + pick(2) + getx(3).\n\
         ## Main\n    Let ys be [10, 20, 30].\n    Show run(ys).\n",
    );
}

/// CAPTURE OF A FUNCTION-LOCAL-BUILT COMPOSITE — closures over a map and a heterogeneous tuple
/// CONSTRUCTED in the function. Their shapes need the inferred register kinds (map value, tuple
/// positions), so the plan's `reg_shape` is completed post-inference (`complete_reg_shape`); the
/// closure bodies then resolve `item k of m` and `item N of t`.
#[test]
fn aot_closure_captures_local_built_map_and_tuple() {
    assert_aot_matches_treewalker(
        "## To run () -> Int:\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 42.\n    \
         Let t be (10, true, 5).\n    Let lookup be (k: Int) -> item k of m.\n    \
         Let pick be (n: Int) -> n + item 1 of t + item 3 of t.\n    Return lookup(1) + pick(0).\n\
         ## Main\n    Show run().\n",
    );
}

/// CAPTURE OF A FUNCTION-PARAMETER COMPOSITE — a closure closing over its function's struct and enum
/// PARAMETERS. The capture's shape is read from the enclosing plan's unified `reg_shape` (built from
/// the resolved param-seed tracks, `Move`-aliased), so `p's x` (field layout) and `Inspect s` (variant
/// layout) resolve though `p`/`s` are parameters, not built in the function.
#[test]
fn aot_closure_captures_param_struct_and_enum() {
    assert_aot_matches_treewalker(
        "## A Pt has:\n    An x: Int.\n\n\
         ## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## To run (p: Pt, s: Shape) -> Int:\n    Let getx be (n: Int) -> n + p's x.\n    \
         Let area be (n: Int) ->:\n        Inspect s:\n            When Circle (r): Return r + n.\n            \
         When Rectangle (w, h): Return w * h.\n        Return 0.\n    Return getx(10) + area(50).\n\
         ## Main\n    Let q be a new Pt with x 100.\n    Let c be a new Circle with radius 7.\n    Show run(q, c).\n",
    );
}

/// CLOSURE OVER AN ENUM (block body, `Inspect`) and a HETEROGENEOUS TUPLE. The enum's shape comes
/// from its `NewVariant` constructor; the het-tuple literal resolves through `boundary_of_value_expr`
/// (tuple + literal element types). Both seed the capture's shape so the closure body resolves.
#[test]
fn aot_closure_captures_enum_and_tuple() {
    assert_aot_matches_treewalker(
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## Main\nLet s be a new Circle with radius 7.\nLet t be (10, true, 5).\n\
         Let area be (n: Int) ->:\n    Inspect s:\n        When Circle (r): Return r + n.\n        \
         When Rectangle (w, h): Return w * h.\n    Return 0.\n\
         Let pick be (n: Int) -> n + item 1 of t + item 3 of t.\n\
         Show area(100).\n    Show pick(0).\n",
    );
}

/// CLOSURE OVER A COMPOSITE WITH A SHAPE — capturing a struct and a map. The captured global's type
/// is resolved by the compiler (`global_types`) and the AOT seeds the capture's shape via
/// `boundary_to_param_shape`, so `p's x` (field layout) and `item k of m` (value kind) resolve in the
/// closure body — the capture analog of a composite parameter.
#[test]
fn aot_closure_captures_struct_and_map() {
    assert_aot_matches_treewalker(
        "## A Pt has:\n    An x: Int.\n    A y: Int.\n\n\
         ## Main\nLet p be a new Pt with x 3 and y 4.\n\
         Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 99.\n\
         Let getx be (n: Int) -> n + p's x.\n\
         Let lookup be (k: Int) -> p's y + item k of m.\n\
         Show getx(10).\n    Show lookup(1).\n",
    );
}

/// HETEROGENEOUS tuple as a PARAMETER and a RETURN — a mixed-kind tuple is a buffer of per-kind
/// 8-byte slots; `BoundaryType::Tuple` carries the per-position kinds (seeded as `ParamShape::Tuple`
/// for params, from `return_type` at the `Call` for returns), so a constant `item N of t` resolves.
#[test]
fn aot_heterogeneous_tuple_param_and_return() {
    assert_aot_matches_treewalker(
        "## To at1 (t: Triple of Int and Bool and Int) -> Int:\n    Return item 1 of t + item 3 of t.\n\
         ## To mk () -> Pair of Int and Bool:\n    Return (42, true).\n\
         ## Main\n    Let q be (10, false, 5).\n    Show at1(q).\n\
         Let r be mk().\n    Show item 1 of r.\n",
    );
}

/// NESTED CLOSURE — a closure defined inside another closure's body, returning a PURE inner-closure
/// result (`Return inner(i)`). The inner body is emitted INLINE in the parent region and jumped over,
/// so it is unreachable THERE; `infer_result` skips its `Return` (via `pc_reach`) instead of letting
/// the unknown-kind inline `Return` poison the parent's result, and the FIXPOINT planner propagates
/// the inner→outer→caller result chain that one pass could not resolve.
#[test]
fn aot_nested_closure_result() {
    assert_aot_matches_treewalker(
        "## To run () -> Int:\n    Let outer be (i: Int) ->:\n        Let inner be (j: Int) -> j * 2.\n        \
         Return inner(i).\n    Return outer(5) + outer(8).\n## Main\n    Show run().\n",
    );
}

/// NESTED CLOSURE CAPTURES — an inner closure built inside an outer closure's body closes over the
/// outer's PARAM (`i`), the outer's function-LOCAL seq (`xs`) and enum (`s`), and a promoted GLOBAL
/// (`base`). The fixpoint plans the outer first, then reads the inner's capture kinds/shapes from the
/// now-planned outer body's `reg_shape`, so a nested capture resolves like a top-level one; `useEnum`
/// is itself a block-body nested closure that `Inspect`s the captured local enum.
#[test]
fn aot_nested_closure_captures() {
    assert_aot_matches_treewalker(
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## To run () -> Int:\n    Let outer be (i: Int) ->:\n        Let xs be [10, 20, 30].\n        \
         Let s be a new Circle with radius 9.\n        Let useSeq be (j: Int) -> i + base + item j of xs.\n        \
         Let useEnum be (n: Int) ->:\n            Inspect s:\n                When Circle (r): Return r + n.\n                \
         When Rectangle (w, h): Return w * h.\n            Return 0.\n        Return useSeq(1) + useEnum(100).\n    \
         Return outer(5).\n## Main\n    Let base be 1000.\n    Show run().\n",
    );
}

/// CLOSURE AS A RETURN VALUE — a function factory returns a closure capturing its parameter. The call
/// on the returned handle is a `call_indirect` through the closure object's runtime func index; the
/// planner publishes which closure each function returns (`Plan.return_closure`) and seeds
/// `closure_of[Call dst]` from it, so `f(5)` resolves its callee/captures/result like a local closure.
/// Two instances keep distinct captures; `pickerFor` returns a closure over a `Seq` parameter.
#[test]
fn aot_closure_as_return_value() {
    assert_aot_matches_treewalker(
        "## To makeAdder (n: Int) -> Closure:\n    Let add be (x: Int) -> x + n.\n    Return add.\n\
         ## To pickerFor (xs: Seq of Int) -> Closure:\n    Let pick be (i: Int) -> item i of xs.\n    \
         Return pick.\n## Main\n    Let f be makeAdder(10).\n    Let g be makeAdder(100).\n    \
         Let ys be [10, 20, 30].\n    Let p be pickerFor(ys).\n    Show f(5).\n    Show g(5).\n    Show p(2).\n",
    );
}

/// CLOSURE AS AN ARGUMENT — a higher-order function calling a closure parameter (`apply(f,x): Return
/// f(x)`). A whole-program pass attributes each call's closure arguments to the parameters they feed;
/// when every call passes the same closure, the parameter is typed `Kind::Closure` (i32) and its
/// `closure_of` is seeded, so `f(x)` resolves callee/captures/result like a local one. `apply` takes a
/// non-capturing closure at two sites; `useAdder` takes a capturing one that is itself a RETURNED
/// closure (return→pass→call composes).
#[test]
fn aot_closure_as_argument() {
    assert_aot_matches_treewalker(
        "## To apply (f: Closure, x: Int) -> Int:\n    Return f(x).\n\
         ## To useAdder (g: Closure) -> Int:\n    Return g(2) + g(10).\n\
         ## To makeAdder (n: Int) -> Closure:\n    Let add be (x: Int) -> x + n.\n    Return add.\n\
         ## Main\n    Let dbl be (m: Int) -> m * 2.\n    Show apply(dbl, 21).\n    Show apply(dbl, 9).\n    \
         Let made be makeAdder(100).\n    Show useAdder(made).\n",
    );
}

/// FLOAT CLOSURES — a closure with a `Float` parameter / capture. Closures hardcode all-Int
/// `param_kinds` (the VM/JIT entry-guard contract), so a `(n: Float)` closure used to mis-size its
/// WASM signature (i64 param vs f64 argument). The closure now records its declared `param_types`
/// (additive — AOT-only) and both seed paths honor them. `half` is non-capturing; `addK` captures a
/// `Float`; `apply` takes a float closure as an argument.
#[test]
fn aot_closure_float_param_and_capture() {
    assert_aot_matches_treewalker(
        "## To apply (f: Closure, x: Float) -> Float:\n    Return f(x).\n\
         ## Main\n    Let half be (n: Float) -> n / 2.0.\n    Let k be 10.0.\n    \
         Let addK be (n: Float) -> n + k.\n    Show half(9.0).\n    Show addK(2.5).\n    \
         Show apply(half, 5.0).\n    Show apply(half, 3.0).\n",
    );
}

/// CLOSURE with a COMPOSITE (struct / Text) PARAMETER passed as an argument. A `(q: Pt)` / `(t: Text)`
/// closure param is an i32 handle; the closure now records its declared `param_types` resolved through
/// the compiler's `user_types` map (structs/enums) — so `q's x` / `length of t` work in the closure
/// body, and the closure-argument path passes the handle at i32. Both higher-order functions are
/// monomorphic (one closure each), so the param-origin pass resolves each callee.
#[test]
fn aot_closure_struct_and_text_param() {
    assert_aot_matches_treewalker(
        "## A Pt has:\n    An x: Int.\n\n## To withPt (g: Closure, p: Pt) -> Int:\n    Return g(p).\n\
         ## To withText (h: Closure, s: Text) -> Int:\n    Return h(s).\n\
         ## Main\n    Let getx be (q: Pt) -> q's x.\n    Let len be (t: Text) -> length of t.\n    \
         Let pt be a new Pt with x 42.\n    Show withPt(getx, pt).\n    Show withText(len, \"hello\").\n",
    );
}

/// CLOSURE COMPOSITION — a closure that captures other closures and calls them. The captured closure
/// value is an i32 handle whose traced body function index flows from the build site, so the body can
/// `call_indirect` it. `twice` captures a function-LOCAL closure (`add1`); `comp` captures GLOBAL
/// closures (`dbl`/`inc`, Main `Let` closures promoted to globals, resolved via `global_closures`).
#[test]
fn aot_closure_composition() {
    assert_aot_matches_treewalker(
        "## To run () -> Int:\n    Let add1 be (n: Int) -> n + 1.\n    \
         Let twice be (n: Int) -> add1(add1(n)).\n    Return twice(10).\n\
         ## Main\n    Let dbl be (n: Int) -> n * 2.\n    Let inc be (n: Int) -> n + 1.\n    \
         Let comp be (n: Int) -> dbl(inc(n)).\n    Show run().\n    Show comp(10).\n    Show comp(20).\n",
    );
}

/// STRUCT with a MAP / ENUM field accessed on a LOCALLY-BUILT struct — `item k of (s's mapfield)` and
/// `Inspect s's enumfield` resolve because the locally-built `GetField` now re-seeds the field result's
/// value kind / variant layout from the struct's declared type (previously only cross-region structs did).
#[test]
fn aot_struct_map_and_enum_field() {
    assert_aot_matches_treewalker(
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\
         ## A Reg has:\n    A counts: Map of Int to Int.\n    A shape: Shape.\n\n\
         ## Main\n    Let mutable c be a new Map of Int to Int.\n    Set item 1 of c to 42.\n    \
         Set item 2 of c to 8.\n    Let s be a new Circle with radius 9.\n    \
         Let r be a new Reg with counts c and shape s.\n    \
         Show item 1 of (r's counts) + item 2 of (r's counts).\n    Inspect r's shape:\n        \
         When Circle (rad): Show rad.\n        When Square (sd): Show sd.\n",
    );
}

/// A `Seq of Bool` — built by `Push false`, mutated by `Set item N to true`, read by `item N equals
/// true`. Booleans ride the `SeqInt` representation (an `i64` 0/1 per 8-byte slot), so the whole
/// sieve-of-Eratosthenes flag-array pattern compiles. (The element is never `Show`n as a bool.)
#[test]
fn aot_seq_of_bool() {
    assert_aot_matches_treewalker(
        "## Main\n    Let mutable flags be a new Seq of Bool.\n    Let mutable i be 0.\n    \
         While i is less than 5:\n        Push false to flags.\n        Set i to i + 1.\n    \
         Set item 2 of flags to true.\n    Set item 4 of flags to true.\n    Let mutable trues be 0.\n    \
         Set i to 1.\n    While i is at most 5:\n        If item i of flags equals true:\n            \
         Set trues to trues + 1.\n        Set i to i + 1.\n    Show trues.\n",
    );
}

/// STORE-SIDE aliasing value semantics: a struct field constructed from a variable shares that
/// variable's sequence, so mutating the variable afterwards must copy-on-write (the field keeps the
/// original). `xs` grows to 3; `b's items` stays 2.
#[test]
fn aot_struct_field_shares_source_then_cow() {
    assert_aot_matches_treewalker(
        "## A Bag has:\n    An items: Seq of Int.\n\n\
         ## Main\n    Let xs be [10, 20].\n    Let b be a new Bag with items xs.\n    \
         Push 99 to xs.\n    Show length of xs.\n    Show length of b's items.\n",
    );
}

/// STORE-SIDE aliasing through a MAP VALUE: a sequence stored as a map value is shared, and reading
/// it back then mutating the original must copy-on-write. `inner` grows to 3; the map's value stays 2.
#[test]
fn aot_map_value_shares_source_then_cow() {
    assert_aot_matches_treewalker(
        "## Main\n    Let inner be [10, 20].\n    Let mutable m be a new Map of Int to Seq of Int.\n    \
         Set item 1 of m to inner.\n    Push 99 to inner.\n    \
         Show length of inner.\n    Show length of item 1 of m.\n",
    );
}

/// STORE-SIDE aliasing via `Set item k of ... to <handle>`: overwriting an element with a variable
/// shares it, so mutating the variable afterwards must copy-on-write. `inner` grows to 3; the stored
/// element stays 2.
#[test]
fn aot_set_index_element_shares_source_then_cow() {
    assert_aot_matches_treewalker(
        "## Main\n    Let inner be [10, 20].\n    Let mutable outer be a new Seq of Seq of Int.\n    \
         Push [1] to outer.\n    Set item 1 of outer to inner.\n    Push 99 to inner.\n    \
         Show length of inner.\n    Show length of item 1 of outer.\n",
    );
}

/// STORE-SIDE aliasing: pushing a sequence AS AN ELEMENT of a sequence-of-sequences shares it, so a
/// later mutation of the original must copy-on-write. `inner` grows to 3; the stored row stays 2.
#[test]
fn aot_seq_element_shares_source_then_cow() {
    assert_aot_matches_treewalker(
        "## Main\n    Let inner be [10, 20].\n    Let mutable outer be a new Seq of Seq of Int.\n    \
         Push inner to outer.\n    Push 99 to inner.\n    Show length of inner.\n    \
         Show length of item 1 of outer.\n",
    );
}

/// Indexing a `Text` (`item i of text`) yields a one-character `Text`, so a substring search that
/// compares `item (i+j) of text` to `item (j+1) of needle` byte-by-byte compiles and FINDS the
/// match — here "XYZ" inside "abXYZcd" at position 3, so the count is 1.
#[test]
fn aot_text_index_substring_search() {
    assert_aot_matches_treewalker(
        "## Main\n    Let text be \"abXYZcd\".\n    Let needle be \"XYZ\".\n    Let needleLen be 3.\n    \
         Let textLen be length of text.\n    Let mutable count be 0.\n    Let mutable i be 1.\n    \
         While i is at most textLen - needleLen + 1:\n        Let mutable matched be 1.\n        \
         Let mutable j be 0.\n        While j is less than needleLen:\n            \
         If item (i + j) of text is not item (j + 1) of needle:\n                Set matched to 0.\n                \
         Set j to needleLen.\n            Set j to j + 1.\n        If matched equals 1:\n            \
         Set count to count + 1.\n        Set i to i + 1.\n    Show count.\n",
    );
}

/// `//` floor division through the WASM backend, across the FULL sign matrix — the load-bearing
/// case is the negative-operand correction (`i64.div_s` truncates toward zero; `//` floors toward
/// -inf), which the hand-emitted `q - ((r != 0) & ((r ^ b) < 0))` sequence must get bit-exact
/// against the tree-walker. A silent miscompile here is exactly the catastrophic integer bug.
#[test]
fn aot_floordiv_integer_sign_matrix() {
    for src in [
        "## Main\n    Show 7 // 2.\n",
        "## Main\n    Show 8 // 3.\n",
        "## Main\n    Show -7 // 2.\n",
        "## Main\n    Show 7 // -2.\n",
        "## Main\n    Show -7 // -2.\n",
        "## Main\n    Show 0 // 5.\n",
        "## Main\n    Show -1 // 5.\n",
        "## Main\n    Show 10 // 3 // 2.\n",
        "## Main\n    Show 10 - 8 // 2.\n",
        "## Main\n    Let a be -17.\n    Let b be 5.\n    Show a // b.\n",
    ] {
        assert_aot_matches_treewalker(src);
    }
}

/// `//` on Float operands floors the quotient but stays Float (`f64.floor(a / b)`), agreeing with
/// the tree-walker for both signs.
#[test]
fn aot_floordiv_float() {
    for src in [
        "## Main\n    Show 7.5 // 2.0.\n",
        "## Main\n    Show -7.5 // 2.0.\n",
    ] {
        assert_aot_matches_treewalker(src);
    }
}
