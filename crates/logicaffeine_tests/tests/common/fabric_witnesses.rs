//! The fabric equivalence-witness registry — the `eval == VM` batteries, the rich corpus and the
//! whole-program corpus that weld the 117-op fabric coverage claim to its proof. Shared by `#[path]`
//! between `fabric_coverage_lock` (the fabric column) and `rdu_op_coverage_lock` (the RDU column, the
//! same witnesses on the SN50 descriptor's envelope), so the two columns can never drift apart.
#![allow(dead_code)]

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, eval_decision_fabric, eval_decision_fabric_heap,
    eval_loop_fabric_seq, eval_whole_program, loop_fabric_from_source,
    whole_program_fabric_from_program, whole_program_fabric_from_source,
};
use logicaffeine_compile::compile::{program_outcome_scheduled, vm_outcome, vm_outcome_concurrent};
use logicaffeine_compile::semantics::builtins::BuiltinId;
use logicaffeine_compile::vm::compiler::magic_u64_gen;
use logicaffeine_compile::vm::{
    BoundaryType, ChanElem, CompiledProgram, Constant, Op, StructTypeDef, Vm,
};

/// The straight-line arithmetic witness program: `## To <name> (a, b) -> Int: Return <expr>.`
pub fn arith_source(name: &str, expr: &str) -> String {
    format!("## To {name} (a: Int, b: Int) -> Int:\n    Return {expr}.\n")
}

/// The comparison witness program (single-return, if-converted to a Select): `r=0; if a <cmp> b: r=1; return r`.
pub fn cmp_source(name: &str, cmp: &str) -> String {
    format!(
        "## To {name} (a: Int, b: Int) -> Int:\n    Let mutable r be 0.\n    If a {cmp} b:\n        Set r to 1.\n    Return r.\n"
    )
}

/// The source-authored two-argument witnesses of the registry, by op: `(op, function name, expression)`
/// for the arithmetic family and `(op, function name, comparison)` for the comparison family — the same
/// programs `FABRIC_EQUIV` runs, listed so a descriptor column can place and evaluate each one.
pub const ARITH_WITNESSES: &[(&str, &str, &str)] = &[
    ("Add", "fab_add", "a + b"),
    ("Sub", "fab_sub", "a - b"),
    ("Mul", "fab_mul", "a * b"),
    ("Div", "fab_div", "a / b"),
    ("Mod", "fab_mod", "a % b"),
    ("FloorDiv", "fab_fdiv", "a // b"),
    ("BitAnd", "fab_and", "a & b"),
    ("BitOr", "fab_or", "a | b"),
    ("BitXor", "fab_xor", "a ^ b"),
    ("Shl", "fab_shl", "a shifted left by b"),
    ("Shr", "fab_shr", "a shifted right by b"),
];
pub const CMP_WITNESSES: &[(&str, &str, &str)] = &[
    ("Lt", "fab_lt", "is less than"),
    ("Gt", "fab_gt", "is greater than"),
    ("LtEq", "fab_le", "is at most"),
    ("GtEq", "fab_ge", "is at least"),
    ("Eq", "fab_eq", "is equal to"),
    ("NotEq", "fab_ne", "is not"),
];

// ── fuzz + VM-oracle helpers ────────────────────────────────────────────────────────────────────────
pub fn mix(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}
pub fn arg(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}
/// Run `name(a, b)` through the VM — the golden every fabric witness is checked against.
pub fn vm_call2(fn_src: &str, name: &str, a: i64, b: i64) -> i64 {
    let src = format!(
        "{fn_src}## Main\n    Show {name}({}, {}).\n",
        arg(a),
        arg(b)
    );
    let out = vm_outcome(&src);
    assert!(
        out.error.is_none(),
        "VM error on {name}({a}, {b}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .unwrap_or_else(|| panic!("{name}: no Show output"))
        .parse::<i64>()
        .unwrap_or_else(|e| panic!("{name}: non-int Show `{}`: {e}", out.output))
}

/// The equivalence witness for a 2-arg Int function: its decision fabric evaluates EXACTLY like the VM over
/// a broad seeded fuzz (small `b` so div/mod/floordiv hit the b==0 trap and every sign combination).
pub fn witness2(name: &str, fn_src: &str) {
    let df = decision_fabric_from_source(fn_src, name);
    let mut st = 0xC0FFEE_1234_u64 ^ (name.bytes().map(|c| c as u64).sum::<u64>());
    for _ in 0..300 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        // b is always NON-ZERO and spans both signs: the ≡VM equivalence domain excludes divide-by-zero,
        // where the VM *errors* ("Division by zero") while the fabric hardware-*traps to 0* — a deliberate,
        // program-erroneous divergence (the trap is verified in FuncUnit::eval, not against the VM).
        let mut b = (mix(&mut st) % 200) as i64 + 1; // 1..=200
        if mix(&mut st) & 1 == 0 {
            b = -b; // → [-200,-1] ∪ [1,200]
        }
        let got = eval_decision_fabric(&df, &[a, b]);
        let want = vm_call2(fn_src, name, a, b);
        assert_eq!(
            got, want,
            "{name} fabric != VM on ({a}, {b}): fabric {got}, vm {want}"
        );
    }
}
/// A straight-line arithmetic witness: `## To <name> (a,b) -> Int: Return a <op> b.`
pub fn witness_arith(name: &str, expr: &str) {
    witness2(name, &arith_source(name, expr));
}
/// A comparison witness (single-return, if-converted to a Select): `r=0; if a <cmp> b: r=1; return r`.
pub fn witness_cmp(name: &str, cmp: &str) {
    witness2(name, &cmp_source(name, cmp));
}

/// The `IterNext` (sequence element-read) witness: a `Repeat for x in xs` fold lowers to a loop fabric with
/// a `MemRead` tile reading `xs[iv]`; its evaluation equals the VM EXACTLY over random-length seqs (incl.
/// the empty seq → zero iterations). This is the memory-read tile proven by `fabric_mem_lock`.
pub fn witness_iternext() {
    const SEQSUM: &str = "## To seqsum (xs: Seq of Int) -> Int:\n    Let mutable acc be 0.\n    Repeat for x in xs:\n        Set acc to acc + x.\n    Return acc.\n";
    let lf = loop_fabric_from_source(SEQSUM, "seqsum");
    let params = vec![0i64; lf.n_in];
    let mut st = 0x5EED_C0DE_1234_u64;
    for _ in 0..250 {
        let n = (mix(&mut st) as usize) % 13; // 0..=12, includes the empty sequence
        let seq: Vec<i64> = (0..n)
            .map(|_| (mix(&mut st) % 4000) as i64 - 2000)
            .collect();
        let got = eval_loop_fabric_seq(&lf, &params, &seq);
        // VM golden: fold the same seq literal.
        let list = seq.iter().map(|e| arg(*e)).collect::<Vec<_>>().join(", ");
        let build = if seq.is_empty() {
            "a new Seq of Int".to_string()
        } else {
            format!("[{list}]")
        };
        let src = format!("{SEQSUM}## Main\n    Let xs be {build}.\n    Show seqsum(xs).\n");
        let out = vm_outcome(&src);
        assert!(
            out.error.is_none(),
            "VM error on seqsum({seq:?}): {:?}",
            out.error
        );
        let want: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            got, want,
            "IterNext (seq element read) fabric != VM on {seq:?}: fabric {got}, vm {want}"
        );
    }
}

/// Fold `seq` into a heap laid out exactly like the VM: pointer 0 = `[len, e1..eN]`.
pub fn seq_heap(seq: &[i64]) -> Vec<i64> {
    let mut heap = vec![seq.len() as i64];
    heap.extend_from_slice(seq);
    heap
}
pub fn seq_literal(seq: &[i64]) -> String {
    if seq.is_empty() {
        "a new Seq of Int".to_string()
    } else {
        format!(
            "[{}]",
            seq.iter().map(|e| arg(*e)).collect::<Vec<_>>().join(", ")
        )
    }
}

/// `Length` witness — `Return length of xs.` lowers to `HeapRead(ptr)`; the fabric reads `heap[0]` (the
/// length header) and equals the VM over random-length seqs (including the empty seq).
pub fn witness_length() {
    const SRC: &str = "## To fab_len (xs: Seq of Int) -> Int:\n    Return length of xs.\n";
    let df = decision_fabric_from_source(SRC, "fab_len");
    let mut st = 0xB0BA_CAFE_u64;
    for _ in 0..200 {
        let n = (mix(&mut st) as usize) % 13;
        let seq: Vec<i64> = (0..n)
            .map(|_| (mix(&mut st) % 4000) as i64 - 2000)
            .collect();
        let got = eval_decision_fabric_heap(&df, &[0], &mut seq_heap(&seq)); // input 0 = xs pointer = 0
        let vmsrc = format!(
            "{SRC}## Main\n    Let xs be {}.\n    Show fab_len(xs).\n",
            seq_literal(&seq)
        );
        let out = vm_outcome(&vmsrc);
        assert!(out.error.is_none(), "VM error (length): {:?}", out.error);
        let want: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            got,
            want,
            "Length fabric != VM on len {}: fabric {got}, vm {want}",
            seq.len()
        );
    }
}

/// `Index` witness — `Return item i of xs.` lowers to `HeapRead(ptr + i)` (1-based); the fabric reads
/// `heap[i]` and equals the VM over random non-empty seqs at random in-bounds indices.
pub fn witness_index() {
    const SRC: &str = "## To fab_idx (xs: Seq of Int, i: Int) -> Int:\n    Return item i of xs.\n";
    let df = decision_fabric_from_source(SRC, "fab_idx");
    let mut st = 0x1DE4_BEEF_u64;
    for _ in 0..200 {
        let n = 1 + (mix(&mut st) as usize) % 12; // non-empty
        let seq: Vec<i64> = (0..n)
            .map(|_| (mix(&mut st) % 4000) as i64 - 2000)
            .collect();
        let i = 1 + (mix(&mut st) as usize) % n; // 1-based, in-bounds [1, n]
        let got = eval_decision_fabric_heap(&df, &[0, i as i64], &mut seq_heap(&seq)); // inputs: [xs_ptr=0, i]
        let vmsrc = format!(
            "{SRC}## Main\n    Let xs be [{}].\n    Show fab_idx(xs, {i}).\n",
            seq.iter().map(|e| arg(*e)).collect::<Vec<_>>().join(", ")
        );
        let out = vm_outcome(&vmsrc);
        assert!(out.error.is_none(), "VM error (index): {:?}", out.error);
        let want: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            got, want,
            "Index fabric != VM on seq[{i}] (len {n}): fabric {got}, vm {want}"
        );
    }
}

/// `NewList` witness — `Let ys be [a,b,c]. Return item i of ys.` composes `HeapAlloc` + element `HeapWrite`s
/// into a FRESH list (top of heap, no aliasing → sound without COW); the read-back equals the VM.
pub fn witness_newlist() {
    const SRC: &str = "## To fab_new (a: Int, b: Int, c: Int, i: Int) -> Int:\n    Let ys be [a, b, c].\n    Return item i of ys.\n";
    let df = decision_fabric_from_source(SRC, "fab_new");
    let mut st = 0xADD_115_7_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let c = (mix(&mut st) % 4000) as i64 - 2000;
        let i = 1 + (mix(&mut st) % 3) as i64; // 1..=3
        let mut heap: Vec<i64> = Vec::new(); // empty — NewList allocates the fresh list
        let got = eval_decision_fabric_heap(&df, &[a, b, c, i], &mut heap);
        let vmsrc = format!(
            "{SRC}## Main\n    Show fab_new({}, {}, {}, {i}).\n",
            arg(a),
            arg(b),
            arg(c)
        );
        let out = vm_outcome(&vmsrc);
        assert!(out.error.is_none(), "VM error (newlist): {:?}", out.error);
        let want: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            got, want,
            "NewList fabric != VM on [{a},{b},{c}][{i}]: fabric {got}, vm {want}"
        );
    }
}

/// `SetIndex` witness — `Let mutable ys be [a,b,c]. Set item i of ys to v. Return item i of ys.` composes an
/// in-place `HeapWrite`; the mutated read-back equals the VM (fresh owned list → no COW).
pub fn witness_setindex() {
    const SRC: &str = "## To fab_set (a: Int, b: Int, c: Int, i: Int, v: Int) -> Int:\n    Let mutable ys be [a, b, c].\n    Set item i of ys to v.\n    Return item i of ys.\n";
    let df = decision_fabric_from_source(SRC, "fab_set");
    let mut st = 0x5E7_1DE4_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let c = (mix(&mut st) % 4000) as i64 - 2000;
        let i = 1 + (mix(&mut st) % 3) as i64;
        let v = (mix(&mut st) % 4000) as i64 - 2000;
        let mut heap: Vec<i64> = Vec::new();
        let got = eval_decision_fabric_heap(&df, &[a, b, c, i, v], &mut heap);
        let vmsrc = format!(
            "{SRC}## Main\n    Show fab_set({}, {}, {}, {i}, {}).\n",
            arg(a),
            arg(b),
            arg(c),
            arg(v)
        );
        let out = vm_outcome(&vmsrc);
        assert!(out.error.is_none(), "VM error (setindex): {:?}", out.error);
        let want: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            got, want,
            "SetIndex fabric != VM: set [{a},{b},{c}][{i}]={v}, fabric {got}, vm {want}"
        );
    }
}

/// `ListPush` witness — `Let mutable ys be [a,b]. Push v to ys. Return item 3 of ys.` grows the list (a
/// `HeapWrite` past the old end + a header bump); the pushed element reads back equal to the VM.
pub fn witness_listpush() {
    const SRC: &str = "## To fab_push (a: Int, b: Int, v: Int) -> Int:\n    Let mutable ys be [a, b].\n    Push v to ys.\n    Return item 3 of ys.\n";
    let df = decision_fabric_from_source(SRC, "fab_push");
    let mut st = 0xD0_17_A11_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let v = (mix(&mut st) % 4000) as i64 - 2000;
        let mut heap: Vec<i64> = Vec::new();
        let got = eval_decision_fabric_heap(&df, &[a, b, v], &mut heap);
        let vmsrc = format!(
            "{SRC}## Main\n    Show fab_push({}, {}, {}).\n",
            arg(a),
            arg(b),
            arg(v)
        );
        let out = vm_outcome(&vmsrc);
        assert!(out.error.is_none(), "VM error (listpush): {:?}", out.error);
        let want: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            got, want,
            "ListPush fabric != VM: push {v} onto [{a},{b}], fabric {got}, vm {want}"
        );
    }
}

/// Run a HAND-AUTHORED bytecode program on both the VM oracle and the whole-program fabric, returning their
/// output tapes. The reversible ALU ops (`RevXor`/`RevNeg`/`RevAdd`/`RevSub`) are real ISA ops the soft-core
/// executes but NO LOGOS source emits (they are the `rev_alu_core` primitives), so — exactly as `rtl.rs`
/// does for the scheduler-only ops — their fabric tile is proven by building a raw `CompiledProgram` and
/// differencing the fabric tape against the VM (`Vm::new(&prog).run(); vm.output()`), never a hand golden.
pub fn hand_bytecode_tapes(
    constants: Vec<Constant>,
    code: Vec<Op>,
    register_count: usize,
) -> (Vec<String>, Vec<String>) {
    hand_bytecode_tapes_prog(CompiledProgram {
        constants,
        code,
        register_count,
        ..Default::default()
    })
}
/// The same, for a program that carries extra context (e.g. `struct_types` for a `CrdtJoin` witness): run it
/// on the plain VM oracle and the whole-program fabric, returning both tapes.
pub fn hand_bytecode_tapes_prog(prog: CompiledProgram) -> (Vec<String>, Vec<String>) {
    let mut vm = Vm::new(&prog);
    vm.run().expect("hand-bytecode VM run");
    let want: Vec<String> = vm.output().lines().map(str::to_string).collect();
    let wpf = whole_program_fabric_from_program(&prog);
    let got = eval_whole_program(&wpf);
    (got, want)
}

/// The scheduler-driven variant: the oracle is `program_outcome_scheduled` (RevCoop), for ops that BLOCK on
/// the plain VM (`ChanTryRecv` and friends need the scheduler driver). Fabric side is unchanged.
pub fn hand_bytecode_tapes_sched(
    constants: Vec<Constant>,
    code: Vec<Op>,
    register_count: usize,
) -> (Vec<String>, Vec<String>) {
    let prog = CompiledProgram {
        constants,
        code,
        register_count,
        ..Default::default()
    };
    let oracle = program_outcome_scheduled(&prog);
    assert!(
        oracle.error.is_none(),
        "scheduled oracle error: {:?}",
        oracle.error
    );
    let want: Vec<String> = oracle.output.lines().map(str::to_string).collect();
    let wpf = whole_program_fabric_from_program(&prog);
    let got = eval_whole_program(&wpf);
    (got, want)
}

/// `ChanTryRecv` witness — non-blocking receive on a NON-EMPTY channel returns the front value (≡ a
/// `ChanRecv` dequeue). Enqueue `a` via `ChanTrySend`, `ChanTryRecv` it back, show it — ≡ the scheduler.
pub fn witness_chan_try_recv() {
    let mut st = 0xC0A_72EC_11_u64;
    for _ in 0..100 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        let (got, want) = hand_bytecode_tapes_sched(
            vec![Constant::Int(a)],
            vec![
                Op::ChanNew {
                    dst: 0,
                    cap: -1,
                    elem: ChanElem::Int,
                },
                Op::LoadConst { dst: 1, idx: 0 },
                Op::ChanTrySend {
                    dst: 2,
                    chan: 0,
                    val: 1,
                }, // enqueue a
                Op::ChanTryRecv { dst: 3, chan: 0 }, // dequeue a
                Op::Show { src: 3 },
                Op::Halt,
            ],
            4,
        );
        assert_eq!(got, want, "ChanTryRecv fabric != scheduler on ({a})");
    }
}

/// The reversible CRDT LATTICE JOIN witness: build a `Counter{total}` target and source, `CrdtJoin` them
/// (per-field MAX), read `total`. No source emits `CrdtJoin`, so the program is hand-built WITH its
/// `struct_types` (the field-slot map the fabric resolves through) and differenced against the VM.
pub fn witness_crdt_join() {
    let mut st = 0xC0D_10_11_2_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        let b = (mix(&mut st) % 20_000) as i64 - 10_000;
        let prog = CompiledProgram {
            constants: vec![
                Constant::Text("Counter".into()),
                Constant::Text("total".into()),
                Constant::Int(a),
                Constant::Int(b),
            ],
            struct_types: vec![StructTypeDef {
                name: "Counter".into(),
                fields: vec![("total".into(), BoundaryType::Int)],
            }],
            code: vec![
                Op::NewStruct {
                    dst: 0,
                    type_name: 0,
                }, // target Counter
                Op::LoadConst { dst: 1, idx: 2 },
                Op::StructInsert {
                    obj: 0,
                    field: 1,
                    value: 1,
                }, // target.total = a
                Op::NewStruct {
                    dst: 2,
                    type_name: 0,
                }, // source Counter
                Op::LoadConst { dst: 3, idx: 3 },
                Op::StructInsert {
                    obj: 2,
                    field: 1,
                    value: 3,
                }, // source.total = b
                Op::CrdtJoin {
                    target: 0,
                    source: 2,
                }, // target.total = max(a, b)
                Op::GetField {
                    dst: 4,
                    obj: 0,
                    field: 1,
                },
                Op::Show { src: 4 },
                Op::Halt,
            ],
            register_count: 5,
            ..Default::default()
        };
        let (got, want) = hand_bytecode_tapes_prog(prog);
        assert_eq!(got, want, "CrdtJoin fabric != VM on max({a}, {b})");
    }
}

/// A binary reversible-ALU witness: `R0 = a; R1 = b; R0 <op> R1; Show R0` — fabric tape ≡ VM over fuzz.
pub fn witness_rev_binary(seed: u64, mk: fn(u16, u16) -> Op, label: &str) {
    let mut st = seed;
    for _ in 0..200 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        let b = (mix(&mut st) % 20_000) as i64 - 10_000;
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(a), Constant::Int(b)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                mk(0, 1),
                Op::Show { src: 0 },
                Op::Halt,
            ],
            2,
        );
        assert_eq!(got, want, "{label} fabric != VM on ({a}, {b})");
    }
}

/// The unary reversible negate witness: `R0 = a; R0 = -R0; Show R0` — fabric tape ≡ VM over fuzz.
pub fn witness_rev_neg() {
    let mut st = 0x0E6_2222_u64.wrapping_mul(0x9E37_79B9);
    for _ in 0..200 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(a)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::RevNeg { place: 0 },
                Op::Show { src: 0 },
                Op::Halt,
            ],
            1,
        );
        assert_eq!(got, want, "RevNeg fabric != VM on ({a})");
    }
}

/// `IndexUnchecked` witness — the Oracle-proven-in-bounds read. No source emits it on the no-oracle fabric
/// path, so hand-build `[a,b,c]` then `IndexUnchecked item i` (in range) and difference vs the VM.
pub fn witness_index_unchecked() {
    let mut st = 0x1DE4_11_00_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let c = (mix(&mut st) % 4000) as i64 - 2000;
        let i = 1 + (mix(&mut st) % 3) as i64; // 1..=3, in-bounds
        let (got, want) = hand_bytecode_tapes(
            vec![
                Constant::Int(a),
                Constant::Int(b),
                Constant::Int(c),
                Constant::Int(i),
            ],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::LoadConst { dst: 2, idx: 2 },
                Op::NewList {
                    dst: 3,
                    start: 0,
                    count: 3,
                },
                Op::LoadConst { dst: 4, idx: 3 },
                Op::IndexUnchecked {
                    dst: 5,
                    collection: 3,
                    index: 4,
                },
                Op::Show { src: 5 },
                Op::Halt,
            ],
            6,
        );
        assert_eq!(
            got, want,
            "IndexUnchecked fabric != VM on [{a},{b},{c}][{i}]"
        );
    }
}

/// `SetIndexUnchecked` witness — the in-place write, Oracle-proven-in-bounds. Hand-build `[a,b,c]`, set item
/// `i` to `v` unchecked, read it back — the mutated read ≡ VM.
pub fn witness_set_index_unchecked() {
    let mut st = 0x5E7_11_00_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let c = (mix(&mut st) % 4000) as i64 - 2000;
        let i = 1 + (mix(&mut st) % 3) as i64;
        let v = (mix(&mut st) % 4000) as i64 - 2000;
        let (got, want) = hand_bytecode_tapes(
            vec![
                Constant::Int(a),
                Constant::Int(b),
                Constant::Int(c),
                Constant::Int(i),
                Constant::Int(v),
            ],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::LoadConst { dst: 2, idx: 2 },
                Op::NewList {
                    dst: 3,
                    start: 0,
                    count: 3,
                },
                Op::LoadConst { dst: 4, idx: 3 },
                Op::LoadConst { dst: 5, idx: 4 },
                Op::SetIndexUnchecked {
                    collection: 3,
                    index: 4,
                    value: 5,
                },
                Op::IndexUnchecked {
                    dst: 6,
                    collection: 3,
                    index: 4,
                },
                Op::Show { src: 6 },
                Op::Halt,
            ],
            7,
        );
        assert_eq!(
            got, want,
            "SetIndexUnchecked fabric != VM: [{a},{b},{c}][{i}]={v}"
        );
    }
}

/// `ExactDiv` witness — the Oracle emits it only when the divisor divides EXACTLY, so build exact operands
/// (`lhs = rhs * q`) and difference the truncating fabric divider against the VM's exact quotient.
pub fn witness_exact_div() {
    let mut st = 0xE7AC_1D14_u64;
    for _ in 0..200 {
        let mut rhs = (mix(&mut st) % 400) as i64 - 200; // [-200,199]
        if rhs == 0 {
            rhs = 1;
        }
        let q = (mix(&mut st) % 4000) as i64 - 2000;
        let lhs = rhs * q; // EXACT: rhs | lhs
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(lhs), Constant::Int(rhs)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::ExactDiv {
                    dst: 2,
                    lhs: 0,
                    rhs: 1,
                },
                Op::Show { src: 2 },
                Op::Halt,
            ],
            3,
        );
        assert_eq!(got, want, "ExactDiv fabric != VM on {lhs} / {rhs} (= {q})");
    }
}

/// `DivPow2` witness — `lhs / 2^k` (truncating Div by the constant `2^k`). Hand-build over random `lhs` and
/// small `k`, difference vs the VM (which computes exactly this).
pub fn witness_div_pow2() {
    let mut st = 0xD12_2_C0DE_u64;
    for _ in 0..200 {
        let lhs = (mix(&mut st) % 200_000) as i64 - 100_000;
        let k = (1 + (mix(&mut st) % 10)) as u8; // 1..=10
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(lhs)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::DivPow2 { dst: 1, lhs: 0, k },
                Op::Show { src: 1 },
                Op::Halt,
            ],
            2,
        );
        assert_eq!(got, want, "DivPow2 fabric != VM on {lhs} / 2^{k}");
    }
}

/// `MagicDivU` witness — division/modulo of a NON-NEGATIVE dividend by a constant via the magic reciprocal.
/// For each fuzzed `x` and divisor `c`, generate the real magic parameters (`magic_u64_gen`, the same the
/// compiler emits) for both the quotient (`mul_back = 0`) and remainder (`mul_back = c`) forms, and
/// difference the fabric's gate-level reconstruction against the VM's `magic_eval`.
pub fn witness_magic_div_u() {
    let divisors: [u64; 8] = [3, 5, 6, 7, 10, 100, 1000, 7919];
    let mut st = 0x3A61C_D14_u64;
    for _ in 0..240 {
        let x = (mix(&mut st) % 2_000_000) as i64; // non-negative — MagicDivU's Oracle contract
        let c = divisors[(mix(&mut st) as usize) % divisors.len()];
        let is_mod = mix(&mut st) & 1 == 0;
        let (magic, more) = magic_u64_gen(c);
        let mul_back = if is_mod { c as i64 } else { 0 };
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(x)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::MagicDivU {
                    dst: 1,
                    lhs: 0,
                    magic,
                    more,
                    mul_back,
                    divisor: c as i64,
                    wrap: false,
                },
                Op::Show { src: 1 },
                Op::Halt,
            ],
            2,
        );
        assert_eq!(
            got,
            want,
            "MagicDivU fabric != VM on {x} {} {c}",
            if is_mod { "%" } else { "/" }
        );
    }
}

/// `NewEmptyListI32` witness — the half-width empty `Seq of Int`. Hand-build one, push two values, read the
/// length and the first element back — ≡ VM (identical to the full-width empty list at the fabric heap level).
pub fn witness_new_empty_list_i32() {
    witness_empty_list_allocation(Op::NewEmptyListI32 { dst: 0 });
}

/// Use explicit bytecode so a source-level fixed-buffer optimization cannot erase
/// the ordinary allocation opcode from the coverage proof.
pub fn witness_new_empty_list() {
    witness_empty_list_allocation(Op::NewEmptyList { dst: 0 });
}

fn witness_empty_list_allocation(allocation: Op) {
    let mut st = 0x132_E115_7_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(a), Constant::Int(b), Constant::Int(1)],
            vec![
                allocation.clone(),
                Op::LoadConst { dst: 1, idx: 0 },
                Op::ListPush { list: 0, value: 1 },
                Op::LoadConst { dst: 2, idx: 1 },
                Op::ListPush { list: 0, value: 2 },
                Op::Length {
                    dst: 3,
                    collection: 0,
                }, // 2
                Op::LoadConst { dst: 4, idx: 2 }, // index 1
                Op::Index {
                    dst: 5,
                    collection: 0,
                    index: 4,
                }, // item 1 = a
                Op::Add {
                    dst: 6,
                    lhs: 3,
                    rhs: 5,
                }, // 2 + a
                Op::Show { src: 6 },
                Op::Halt,
            ],
            7,
        );
        assert_eq!(got, want, "NewEmptyListI32 fabric != VM on push({a},{b})");
    }
}

/// Dynamic collection-capacity ops are semantic allocation hints. The fabric's collection
/// storage is fixed at synthesis time, so its only faithful physical lowering is the ordinary
/// constructor. This differential proves that limitation preserves observable VM behavior while
/// the distinct ops remain present in bytecode and coverage traces.
pub fn witness_dynamic_collection_capacity() {
    let (got, want) = hand_bytecode_tapes(
        vec![Constant::Int(17), Constant::Int(9)],
        vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::NewEmptyListWithCapacity {
                dst: 1,
                capacity: 0,
            },
            Op::LoadConst { dst: 2, idx: 1 },
            Op::ListPush { list: 1, value: 2 },
            Op::NewEmptyMapWithCapacity {
                dst: 3,
                capacity: 0,
            },
            Op::NewEmptyListFixed {
                dst: 8,
                capacity: 17,
            },
            Op::ListPush { list: 8, value: 2 },
            Op::Length {
                dst: 5,
                collection: 1,
            },
            Op::Length {
                dst: 6,
                collection: 3,
            },
            Op::Add {
                dst: 7,
                lhs: 5,
                rhs: 6,
            },
            Op::Length {
                dst: 9,
                collection: 8,
            },
            Op::Add {
                dst: 10,
                lhs: 7,
                rhs: 9,
            },
            Op::Show { src: 10 },
            Op::Halt,
        ],
        11,
    );
    assert_eq!(got, want, "dynamic-capacity collection fabric != VM");
}

/// `DestructureTuple` witness — hand-build `(a,b,c)`, unpack into fresh registers, return a combination of
/// the components. (Source emits it only inside a `Repeat for (x,y) in ..` loop.) ≡ VM.
pub fn witness_destructure_tuple() {
    let mut st = 0xD5_7C_711E_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let c = (mix(&mut st) % 4000) as i64 - 2000;
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(a), Constant::Int(b), Constant::Int(c)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::LoadConst { dst: 2, idx: 2 },
                Op::NewTuple {
                    dst: 3,
                    start: 0,
                    count: 3,
                },
                Op::DestructureTuple {
                    src: 3,
                    start: 4,
                    count: 3,
                }, // r4=a, r5=b, r6=c
                Op::Sub {
                    dst: 7,
                    lhs: 4,
                    rhs: 6,
                }, // a - c
                Op::Add {
                    dst: 8,
                    lhs: 7,
                    rhs: 5,
                }, // (a - c) + b
                Op::Show { src: 8 },
                Op::Halt,
            ],
            9,
        );
        assert_eq!(got, want, "DestructureTuple fabric != VM on ({a},{b},{c})");
    }
}

/// `Args` witness — a spatial circuit has no command line, so `Args` is the empty list. Read its length (0)
/// plus a fuzzed constant so the witness stays honest ≡ the no-argument VM run.
pub fn witness_args() {
    let mut st = 0xA265_11_u64;
    for _ in 0..100 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(a)],
            vec![
                Op::Args { dst: 0 },
                Op::Length {
                    dst: 1,
                    collection: 0,
                }, // 0
                Op::LoadConst { dst: 2, idx: 0 },
                Op::Add {
                    dst: 3,
                    lhs: 1,
                    rhs: 2,
                }, // 0 + a = a
                Op::Show { src: 3 },
                Op::Halt,
            ],
            4,
        );
        assert_eq!(got, want, "Args fabric != VM on ({a})");
    }
}

/// `ApproxEq` witness — numeric `a is approximately b` (both to f64, relative+absolute tolerance). The fuzz
/// mixes exactly-equal, off-by-one at LARGE magnitude (where the relative epsilon makes them approx-equal),
/// and unrelated pairs — the fabric's functional unit (which reuses the kernel's exact `logos_approx_eq`) ≡
/// the VM across all. (Float operands are refused at the Float load — the numeric-int case is realized.)
pub fn witness_approx_eq() {
    let src = "## To rich_approx (a: Int, b: Int) -> Int:\n    Let mutable r be 0.\n    If a is approximately b:\n        Set r to 1.\n    Return r.\n";
    let df = decision_fabric_from_source(src, "rich_approx");
    let mut st = 0xAFF_09_2222_u64;
    for _ in 0..300 {
        let a = (mix(&mut st) % 2_000_000_000) as i64 - 1_000_000_000; // up to ±1e9
        let b = match mix(&mut st) % 4 {
            0 => a,                                                     // exactly equal → approx
            1 => a + 1, // off by one: approx iff |a| ≳ 1e9 (relative epsilon)
            2 => a + (mix(&mut st) % 1000) as i64, // off by a little
            _ => (mix(&mut st) % 2_000_000_000) as i64 - 1_000_000_000, // unrelated
        };
        let got = eval_decision_fabric(&df, &[a, b]);
        let want = vm_call2(src, "rich_approx", a, b);
        assert_eq!(got, want, "ApproxEq fabric != VM on ({a}, {b})");
    }
}

/// `EnsureOwned` witness — the COW isolation barrier. Alias a list via `Move`, `EnsureOwned` it, then push
/// to it: the alias must be UNCHANGED (`len == 2`, not 3). This distinguishes the clone from a no-op — a
/// no-op would leak the push into the alias (the fabric's `Move` shares the pointer) and give the wrong
/// answer — so it proves the clone is both required and correct ≡ the VM (whose `Move` is value-semantic).
pub fn witness_ensure_owned() {
    let mut st = 0xE0_C04_11_u64;
    for _ in 0..150 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let c = (mix(&mut st) % 4000) as i64 - 2000;
        let (got, want) = hand_bytecode_tapes(
            vec![
                Constant::Int(a),
                Constant::Int(b),
                Constant::Int(c),
                Constant::Int(1),
            ],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::NewList {
                    dst: 2,
                    start: 0,
                    count: 2,
                }, // r2 = [a, b]
                Op::Move { dst: 3, src: 2 }, // r3 aliases r2
                Op::EnsureOwned { reg: 2 },  // clone r2 → the push can't reach r3
                Op::LoadConst { dst: 4, idx: 2 },
                Op::ListPush { list: 2, value: 4 }, // r2 → [a, b, c]
                Op::LoadConst { dst: 5, idx: 3 },   // index 1
                Op::Index {
                    dst: 6,
                    collection: 3,
                    index: 5,
                }, // r3[1] = a (unchanged)
                Op::Length {
                    dst: 7,
                    collection: 3,
                }, // len r3 = 2 (isolated!)
                Op::Length {
                    dst: 8,
                    collection: 2,
                }, // len r2 = 3 (grew)
                Op::Add {
                    dst: 9,
                    lhs: 6,
                    rhs: 7,
                },
                Op::Add {
                    dst: 10,
                    lhs: 9,
                    rhs: 8,
                },
                Op::Show { src: 10 }, // a + 2 + 3
                Op::Halt,
            ],
            11,
        );
        assert_eq!(got, want, "EnsureOwned fabric != VM on ({a},{b},{c})");
    }
}

/// `FormatValue` witness — the default rendering (no spec/prefix) of a scalar to TEXT: `to_display_string`
/// = int→decimal. Hand-build `FormatValue r0` then `Show` it, comparing the rendered string ≡ the VM's.
pub fn witness_format_value() {
    let mut st = 0xF0_47_11_22_u64;
    for _ in 0..200 {
        let a = (mix(&mut st) % 2_000_000) as i64 - 1_000_000;
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(a)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::FormatValue {
                    dst: 1,
                    src: 0,
                    spec: u32::MAX,
                    debug_prefix: u32::MAX,
                },
                Op::Show { src: 1 },
                Op::Halt,
            ],
            2,
        );
        assert_eq!(got, want, "FormatValue fabric != VM on ({a})");
    }
}

/// `BigIntWide` witness — the 256-bit BigInt lane: `Low(FromInt(a) * FromInt(b))` and `Low(a + b)`. The fuzz
/// SPANS the i64 boundary: many draws OVERFLOW, where the VM promotes to a BigInt and `Low` returns 0 — so a
/// fabric that merely i64-wrapped (returning the low bits instead of 0) is caught. This forces the overflow
/// detection (the pit of success), and the fabric's `big`-flag model ≡ the VM across fit AND overflow.
pub fn witness_bigint_wide() {
    let mut st = 0xB16_11_7_u64;
    // Draw operands biased toward the i64 boundary so Mul/Add overflow is common.
    let draw = |s: &mut u64| -> i64 {
        match mix(s) % 3 {
            0 => (mix(s) % 200_000) as i64 - 100_000, // small → fits
            1 => (mix(s) as i64).wrapping_abs() % 6_000_000_000, // ~3e9 → Mul overflows
            _ => (mix(s) as i64) / 2,                 // near ±i64::MAX/2 → Add/Mul overflow
        }
    };
    for _ in 0..150 {
        let a = draw(&mut st);
        let b = draw(&mut st);
        // Low(a * b)
        let (got, want) = hand_bytecode_tapes_prog(CompiledProgram {
            constants: vec![Constant::Int(a), Constant::Int(b)],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::BigIntWide {
                    kind: 0,
                    dst: 2,
                    a: 0,
                    b: 0,
                }, // FromInt a
                Op::BigIntWide {
                    kind: 0,
                    dst: 3,
                    a: 1,
                    b: 0,
                }, // FromInt b
                Op::BigIntWide {
                    kind: 6,
                    dst: 4,
                    a: 2,
                    b: 3,
                }, // Mul
                Op::BigIntWide {
                    kind: 4,
                    dst: 5,
                    a: 4,
                    b: 0,
                }, // Low
                Op::Show { src: 5 },
                Op::Halt,
            ],
            register_count: 6,
            ..Default::default()
        });
        assert_eq!(got, want, "BigIntWide Low(a*b) fabric != VM on ({a}, {b})");
        // Low(a + b)
        let (got, want) = hand_bytecode_tapes_prog(CompiledProgram {
            constants: vec![Constant::Int(a), Constant::Int(b)],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::BigIntWide {
                    kind: 0,
                    dst: 2,
                    a: 0,
                    b: 0,
                },
                Op::BigIntWide {
                    kind: 0,
                    dst: 3,
                    a: 1,
                    b: 0,
                },
                Op::BigIntWide {
                    kind: 1,
                    dst: 4,
                    a: 2,
                    b: 3,
                }, // Add
                Op::BigIntWide {
                    kind: 4,
                    dst: 5,
                    a: 4,
                    b: 0,
                }, // Low
                Op::Show { src: 5 },
                Op::Halt,
            ],
            register_count: 6,
            ..Default::default()
        });
        assert_eq!(got, want, "BigIntWide Low(a+b) fabric != VM on ({a}, {b})");
    }
}

/// `CallBuiltin` witness — the numeric builtins across BOTH domains. FLOAT LANE: `floor(x * y)` (float mul
/// then the float→int floor bridge) and `floor(sqrt(x))` (float sqrt), over fuzzed floats. INT: `abs(a) +
/// min(a,b) + max(a,b)` over fuzzed ints. Each ≡ the VM (which uses the same f64 IEEE ops / integer ops).
pub fn witness_call_builtin() {
    let mut st = 0xB0117_11_22_u64;
    for _ in 0..120 {
        // float: floor(x * y)
        let x = ((mix(&mut st) % 20_000) as f64 - 10_000.0) / 8.0;
        let y = ((mix(&mut st) % 20_000) as f64 - 10_000.0) / 8.0;
        let (got, want) = hand_bytecode_tapes_prog(CompiledProgram {
            constants: vec![Constant::Float(x), Constant::Float(y)],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::Mul {
                    dst: 2,
                    lhs: 0,
                    rhs: 1,
                }, // float multiply
                Op::CallBuiltin {
                    dst: 3,
                    builtin: BuiltinId::Floor,
                    args_start: 2,
                    arg_count: 1,
                },
                Op::Show { src: 3 },
                Op::Halt,
            ],
            register_count: 4,
            ..Default::default()
        });
        assert_eq!(
            got, want,
            "CallBuiltin floor(x*y) fabric != VM on ({x}, {y})"
        );

        // float: floor(sqrt(x)), x >= 0
        let sx = (mix(&mut st) % 1_000_000) as f64 / 100.0;
        let (got, want) = hand_bytecode_tapes_prog(CompiledProgram {
            constants: vec![Constant::Float(sx)],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::CallBuiltin {
                    dst: 1,
                    builtin: BuiltinId::Sqrt,
                    args_start: 0,
                    arg_count: 1,
                },
                Op::CallBuiltin {
                    dst: 2,
                    builtin: BuiltinId::Floor,
                    args_start: 1,
                    arg_count: 1,
                },
                Op::Show { src: 2 },
                Op::Halt,
            ],
            register_count: 3,
            ..Default::default()
        });
        assert_eq!(
            got, want,
            "CallBuiltin floor(sqrt(x)) fabric != VM on ({sx})"
        );

        // int: abs(a) + min(a,b) + max(a,b). A builtin CLEARS its args, so each call gets FRESH copies.
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        let b = (mix(&mut st) % 20_000) as i64 - 10_000;
        let (got, want) = hand_bytecode_tapes_prog(CompiledProgram {
            constants: vec![Constant::Int(a), Constant::Int(b)],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 }, // args for abs
                Op::CallBuiltin {
                    dst: 2,
                    builtin: BuiltinId::Abs,
                    args_start: 0,
                    arg_count: 1,
                },
                Op::LoadConst { dst: 3, idx: 0 }, // fresh args for min
                Op::LoadConst { dst: 4, idx: 1 },
                Op::CallBuiltin {
                    dst: 5,
                    builtin: BuiltinId::Min,
                    args_start: 3,
                    arg_count: 2,
                },
                Op::LoadConst { dst: 6, idx: 0 }, // fresh args for max
                Op::LoadConst { dst: 7, idx: 1 },
                Op::CallBuiltin {
                    dst: 8,
                    builtin: BuiltinId::Max,
                    args_start: 6,
                    arg_count: 2,
                },
                Op::Add {
                    dst: 9,
                    lhs: 2,
                    rhs: 5,
                },
                Op::Add {
                    dst: 10,
                    lhs: 9,
                    rhs: 8,
                },
                Op::Show { src: 10 }, // abs(a) + min(a,b) + max(a,b)
                Op::Halt,
            ],
            register_count: 11,
            ..Default::default()
        });
        assert_eq!(
            got, want,
            "CallBuiltin abs/min/max fabric != VM on ({a}, {b})"
        );
    }
}

/// `NewCrdt` — create a fresh CRDT value. A counter (kind 4) and PN-counter (kind 5) both start at 0 and
/// render as a plain Int; the replica id the VM draws (`next_replica_id`) is unobservable for a single
/// replica, so the fabric's deterministic initial 0 equals the VM's `Show`. Both counter kinds are checked;
/// the collection kinds (set/seq/register) render as `{}`/`[]` and are a follow-on (the fabric refuses them).
pub fn witness_new_crdt() {
    for kind in [4u8, 5u8] {
        let (got, want) = hand_bytecode_tapes(
            vec![],
            vec![Op::NewCrdt { dst: 0, kind }, Op::Show { src: 0 }, Op::Halt],
            1,
        );
        assert_eq!(
            got, want,
            "NewCrdt(kind={kind}) fresh-counter fabric tape != VM (want [\"0\"])"
        );
    }
}

/// The witness registry — keyed by op mnemonic, one fuzzed `eval == VM` battery each. Its key set MUST
/// equal the compute-tile op set (Test B), so adding a compute tile without its witness goes RED.
#[allow(clippy::type_complexity)]
pub const FABRIC_EQUIV: &[(&str, fn())] = &[
    ("Add", || witness_arith("fab_add", "a + b")),
    ("Sub", || witness_arith("fab_sub", "a - b")),
    ("Mul", || witness_arith("fab_mul", "a * b")),
    ("Div", || witness_arith("fab_div", "a / b")),
    ("Mod", || witness_arith("fab_mod", "a % b")),
    ("FloorDiv", || witness_arith("fab_fdiv", "a // b")),
    ("Lt", || witness_cmp("fab_lt", "is less than")),
    ("Gt", || witness_cmp("fab_gt", "is greater than")),
    ("LtEq", || witness_cmp("fab_le", "is at most")),
    ("GtEq", || witness_cmp("fab_ge", "is at least")),
    ("Eq", || witness_cmp("fab_eq", "is equal to")),
    ("NotEq", || witness_cmp("fab_ne", "is not")),
    ("BitAnd", || witness_arith("fab_and", "a & b")),
    ("BitOr", || witness_arith("fab_or", "a | b")),
    ("BitXor", || witness_arith("fab_xor", "a ^ b")),
    ("Shl", || witness_arith("fab_shl", "a shifted left by b")),
    ("Shr", || witness_arith("fab_shr", "a shifted right by b")),
    // Reversible ALU — hand-authored bytecode (no source emits them), fabric tape ≡ VM oracle.
    ("RevXor", || {
        witness_rev_binary(
            0xADD_1_C0DE,
            |p, d| Op::RevXor { place: p, delta: d },
            "RevXor",
        )
    }),
    ("RevAdd", || {
        witness_rev_binary(
            0xADD_2_C0DE,
            |p, d| Op::RevAdd { place: p, delta: d },
            "RevAdd",
        )
    }),
    ("RevSub", || {
        witness_rev_binary(
            0xADD_3_C0DE,
            |p, d| Op::RevSub { place: p, delta: d },
            "RevSub",
        )
    }),
    ("RevNeg", witness_rev_neg),
    ("CrdtJoin", witness_crdt_join),
    ("NewCrdt", witness_new_crdt),
    ("IndexUnchecked", witness_index_unchecked),
    ("SetIndexUnchecked", witness_set_index_unchecked),
    ("ExactDiv", witness_exact_div),
    ("DivPow2", witness_div_pow2),
    ("MagicDivU", witness_magic_div_u),
    ("NewEmptyList", witness_new_empty_list),
    ("NewEmptyListI32", witness_new_empty_list_i32),
    ("NewEmptyListFixed", witness_dynamic_collection_capacity),
    (
        "NewEmptyListWithCapacity",
        witness_dynamic_collection_capacity,
    ),
    (
        "NewEmptyMapWithCapacity",
        witness_dynamic_collection_capacity,
    ),
    ("ChanTryRecv", witness_chan_try_recv),
    ("DestructureTuple", witness_destructure_tuple),
    ("Args", witness_args),
    ("FormatValue", witness_format_value),
    ("ApproxEq", witness_approx_eq),
    ("EnsureOwned", witness_ensure_owned),
    ("CallBuiltin", witness_call_builtin),
    ("BigIntWide", witness_bigint_wide),
    ("IterNext", witness_iternext),
    ("Length", witness_length),
    ("Index", witness_index),
    ("NewList", witness_newlist),
    ("SetIndex", witness_setindex),
    ("ListPush", witness_listpush),
];

// ── the corpus: rich single-return programs, each threading MANY op needles ───────────────────────────
// This is the LIFT: instead of a bespoke witness per op, a program that already parses and exercises a
// whole op-family is differential-checked (eval == VM) ONCE, and `function_op_names` decodes exactly which
// ops it proves. Indices are LITERAL + in-range so every scalar arg fuzzes unconstrained.
pub struct CorpusFn {
    pub src: &'static str,
    pub fn_name: &'static str,
    pub argc: usize,
}
pub const CORPUS: &[CorpusFn] = &[
    // Lists: build, in-place set, grow, index (×2), length, arithmetic — one program, ~8 needles.
    CorpusFn {
        src: "## To rich_list (a: Int, b: Int, c: Int, v: Int) -> Int:\n    Let mutable xs be [a, b, c].\n    Set item 2 of xs to a * b.\n    Push v to xs.\n    Return item 2 of xs + length of xs + item 4 of xs.\n",
        fn_name: "rich_list",
        argc: 4,
    },
    // Empty-seq construction, then grow by push — proves the empty-list allocator.
    CorpusFn {
        src: "## To rich_empty (a: Int) -> Int:\n    Let mutable ys be a new Seq of Int.\n    Push a to ys.\n    Push a + 1 to ys.\n    Return length of ys + item 1 of ys + item 2 of ys.\n",
        fn_name: "rich_empty",
        argc: 1,
    },
    // Pop the last element, shrink the list — proves ListPop (read-last + header decrement).
    CorpusFn {
        src: "## To rich_pop (a: Int, b: Int, c: Int) -> Int:\n    Let mutable xs be [a, b, c].\n    Pop from xs into last.\n    Return last + length of xs.\n",
        fn_name: "rich_pop",
        argc: 3,
    },
    // Struct: construct (NewStruct + StructInsert×2), mutate a field (StructInsert), read two fields
    // (GetField×2). Proves register TYPE-TRACKING resolves the field NAME to its heap slot ≡ the VM's
    // name-keyed map — the mechanism that also unlocks floats.
    CorpusFn {
        src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## To rich_struct (a: Int, b: Int) -> Int:\n    Let mutable p be a new Point with x a and y b.\n    Set p's x to a * b.\n    Return p's x + p's y.\n",
        fn_name: "rich_struct",
        argc: 2,
    },
    // Enum / pattern matching: construct a payload variant (NewInductive), match it (TestArm), bind the
    // payload (BindArm), return from the matched arm. Proves the tagged-union heap + match-as-Select ≡ VM.
    CorpusFn {
        src: "## A Box is one of:\n    A Full with amount Int.\n    An Empty.\n\n## To rich_enum (a: Int) -> Int:\n    Let b be a new Full with amount a.\n    Inspect b:\n        When Full (v): Return v + 1.\n        When Empty: Return 0.\n",
        fn_name: "rich_enum",
        argc: 1,
    },
    // Compound accumulate — `Set x to x + y` lowers to an AddAssign chain (in-place add).
    CorpusFn {
        src: "## To rich_accum (a: Int, b: Int, c: Int) -> Int:\n    Let mutable total be a.\n    Set total to total + b.\n    Set total to total + c.\n    Set total to total + b.\n    Return total.\n",
        fn_name: "rich_accum",
        argc: 3,
    },
    // Logical not on a Bool value, then a branch on that value — proves `Not` (Select-to-0) + the
    // value-condition path in dec_convert (`If flag` on a non-comparison Bool ≡ `flag != 0`). Single-return
    // assign-then-return form so the whole `If` is inside the decision fabric's extracted body.
    CorpusFn {
        src: "## To rich_not (a: Int) -> Int:\n    Let mutable flag be true.\n    Set flag to not flag.\n    Let mutable r be a + 2.\n    If flag:\n        Set r to a + 1.\n    Return r.\n",
        fn_name: "rich_not",
        argc: 1,
    },
    // Function call — the callee's decision fabric is INLINED (Futamura) into the caller. Proves `Call`
    // composes: rich_call inlines `helper` twice, each seeded with a different argument, ≡ the VM's two calls.
    CorpusFn {
        src: "## To helper (x: Int) -> Int:\n    Return x * x + 1.\n\n## To rich_call (a: Int, b: Int) -> Int:\n    Let p be helper(a).\n    Let q be helper(b).\n    Return p + q.\n",
        fn_name: "rich_call",
        argc: 2,
    },
    // Membership — `contains` unrolls into a bounded OR-scan over the list's static length. `xs contains a`
    // is always true (a ∈ xs); `xs contains v` exercises the miss/hit paths as v fuzzes. Proves Contains ≡ VM.
    CorpusFn {
        src: "## To rich_contains (a: Int, b: Int, c: Int, v: Int) -> Int:\n    Let xs be [a, b, c].\n    Let mutable r be v.\n    If xs contains a:\n        Set r to r + 100.\n    If xs contains v:\n        Set r to r + 10.\n    Return r.\n",
        fn_name: "rich_contains",
        argc: 4,
    },
    // Deep clone `copy of` — proves VALUE SEMANTICS: mutating the clone leaves the original untouched
    // (clone is a fresh heap region). Returns p[1] (unchanged a) + q[1] (mutated to 999) = a + 999 ≡ VM.
    CorpusFn {
        src: "## To rich_clone (a: Int, b: Int, c: Int) -> Int:\n    Let mutable p be [a, b, c].\n    Let mutable q be copy of p.\n    Set item 1 of q to 999.\n    Return item 1 of p + item 1 of q.\n",
        fn_name: "rich_clone",
        argc: 3,
    },
    // Sequence concatenation `followed by` — zs = [a,b] ++ [c,d] = [a,b,c,d]; reads across the seam + length.
    CorpusFn {
        src: "## To rich_concat (a: Int, b: Int, c: Int, d: Int) -> Int:\n    Let xs be [a, b].\n    Let ys be [c, d].\n    Let zs be xs followed by ys.\n    Return item 1 of zs + item 3 of zs + item 4 of zs + length of zs.\n",
        fn_name: "rich_concat",
        argc: 4,
    },
    // Slice `items s through e of xs` — ys = xs[2..=3] = [b, c] (1-based inclusive). Reads + length ≡ VM.
    CorpusFn {
        src: "## To rich_slice (a: Int, b: Int, c: Int, d: Int) -> Int:\n    Let xs be [a, b, c, d].\n    Let ys be items 2 through 3 of xs.\n    Return item 1 of ys + item 2 of ys + length of ys.\n",
        fn_name: "rich_slice",
        argc: 4,
    },
    // Tuple `(a, b, c)` — shares the list heap shape; indexed by `item i of t`. Proves NewTuple ≡ VM.
    CorpusFn {
        src: "## To rich_tuple (a: Int, b: Int, c: Int) -> Int:\n    Let t be (a, b, c).\n    Return item 1 of t + item 3 of t.\n",
        fn_name: "rich_tuple",
        argc: 3,
    },
    // Exponentiation `to the power of` — the sequential power unit. Exponent 3 keeps a³ inside i64 over the
    // ±2000 fuzz range (the VM promotes to bigint on overflow), so the fabric i64 power ≡ the VM value.
    CorpusFn {
        src: "## To rich_pow (a: Int) -> Int:\n    Return a to the power of 3.\n",
        fn_name: "rich_pow",
        argc: 1,
    },
    // Empty set — `length of` an empty set is 0 (the length-0 heap region), so this returns `a`. Proves
    // NewEmptySet + Length-on-empty ≡ VM (the fuzzed `a` keeps the witness honest, not a constant).
    CorpusFn {
        src: "## To rich_eset (a: Int) -> Int:\n    Let s be a new Set of Int.\n    Return length of s + a.\n",
        fn_name: "rich_eset",
        argc: 1,
    },
    // Set DEDUP — `{a, b, a}` adds `a` twice; the set is {a,b} (len 2) or {a} (len 1 if a==b). The repeated
    // `a` GUARANTEES the found=true dedup path fires. Proves SetAdd's runtime-count-guarded scan ≡ VM.
    CorpusFn {
        src: "## To rich_setadd (a: Int, b: Int) -> Int:\n    Let s be {a, b, a}.\n    Return length of s + a.\n",
        fn_name: "rich_setadd",
        argc: 2,
    },
    // Set REMOVAL — remove `a` from {a,b,c}; membership (`contains`) then distinguishes correct compaction
    // (a is gone, b remains) from a no-shift bug (a would still be found). Proves RemoveFrom ≡ VM.
    CorpusFn {
        src: "## To rich_remove (a: Int, b: Int, c: Int) -> Int:\n    Let mutable s be {a, b, c}.\n    Remove a from s.\n    Let mutable r be length of s.\n    If s contains a:\n        Set r to r + 1000.\n    If s contains b:\n        Set r to r + 100.\n    Return r.\n",
        fn_name: "rich_remove",
        argc: 3,
    },
    // Set UNION — {a,b} ∪ {b,c}: b overlaps, so |u| = 3 (distinct) with a,b,c all members. Length + the
    // two membership checks pin the deduped-append size AND contents ≡ VM.
    CorpusFn {
        src: "## To rich_union (a: Int, b: Int, c: Int) -> Int:\n    Let s be {a, b}.\n    Let t be {b, c}.\n    Let u be s union t.\n    Let mutable r be length of u.\n    If u contains a:\n        Set r to r + 100.\n    If u contains c:\n        Set r to r + 10.\n    Return r.\n",
        fn_name: "rich_union",
        argc: 3,
    },
    // Set INTERSECTION — {a,b} ∩ {b,c} = {b} (distinct): |u|=1, b in, a out. Length + membership pin the
    // filter ≡ VM.
    CorpusFn {
        src: "## To rich_intersect (a: Int, b: Int, c: Int) -> Int:\n    Let s be {a, b}.\n    Let t be {b, c}.\n    Let u be s intersection t.\n    Let mutable r be length of u.\n    If u contains a:\n        Set r to r + 100.\n    If u contains b:\n        Set r to r + 10.\n    Return r.\n",
        fn_name: "rich_intersect",
        argc: 3,
    },
    // Interpolation `Concat` — `"n={a}"` renders `a` to decimal and prefixes "n="; its LENGTH = 2 + digit
    // count (+ sign) is Int-comparable, pinning the byte-exact int→decimal + concat ≡ VM (the whole-program
    // corpus verifies the actual string, this pins the length + puts Concat in the proven set).
    CorpusFn {
        src: "## To rich_concat_text (a: Int) -> Int:\n    Let s be \"n={a}\".\n    Return length of s.\n",
        fn_name: "rich_concat_text",
        argc: 1,
    },
    // Empty map — `length of` an empty map is 0 (the length-0 heap region), so this returns `a`. Proves
    // NewEmptyMap + Length-on-empty ≡ VM.
    CorpusFn {
        src: "## To rich_emap (a: Int) -> Int:\n    Let m be a new Map of Int to Int.\n    Return length of m + a.\n",
        fn_name: "rich_emap",
        argc: 1,
    },
    // Reversible register exchange `Swap` — after swapping x,y the difference flips sign, pinning RevSwap ≡ VM.
    CorpusFn {
        src: "## To rich_swap (a: Int, b: Int) -> Int:\n    Let mutable x be a.\n    Let mutable y be b.\n    Swap x and y.\n    Return x - y.\n",
        fn_name: "rich_swap",
        argc: 2,
    },
    // Channel FIFO (sequential) — send a then b, receive in order (x=a, y=b). Proves ChanNew/Send/Recv as a
    // heap FIFO ≡ the scheduler's VecDeque for spawn-free use.
    CorpusFn {
        src: "## To rich_chan (a: Int, b: Int) -> Int:\n    Let ch be a Pipe of Int.\n    Send a into ch.\n    Send b into ch.\n    Receive x from ch.\n    Receive y from ch.\n    Return x - y.\n",
        fn_name: "rich_chan",
        argc: 2,
    },
    // `select` (Await the first of) — a pre-filled channel makes the RECV arm win over the timeout, so the
    // priority resolver binds x = a and runs that arm: r = a + b. Proves SelectArmRecv/SelectArmTimeout/
    // SelectWait (the first-ready-recv-else-timeout state machine + the dst_arm dispatch) ≡ the scheduler.
    CorpusFn {
        src: "## To rich_select (a: Int, b: Int) -> Int:\n    Let ch be a Pipe of Int.\n    Send a into ch.\n    Let mutable r be 0.\n    Await the first of:\n        Receive x from ch:\n            Set r to x + b.\n        After 1 seconds:\n            Set r to b.\n    Return r.\n",
        fn_name: "rich_select",
        argc: 2,
    },
    // Non-blocking `Try send` — always succeeds on the depth-bounded FIFO (enqueue), so the value is then
    // receivable. Proves ChanTrySend ≡ the scheduler.
    CorpusFn {
        src: "## To rich_trysend (a: Int) -> Int:\n    Let ch be a Pipe of Int.\n    Try to send a into ch.\n    Receive x from ch.\n    Return x + 1.\n",
        fn_name: "rich_trysend",
        argc: 1,
    },
    // CRDT counter bump — a `Shared` struct's counter field is `field += amount` (single-replica = running
    // sum). Increase twice, read the total = a + b. Proves CrdtBump ≡ VM via the struct heap model.
    CorpusFn {
        src: "## A Counter is Shared and has:\n    A total: Int.\n\n## To rich_crdt (a: Int, b: Int) -> Int:\n    Let mutable c be a new Counter with total 0.\n    Increase c's total by a.\n    Increase c's total by b.\n    Return c's total.\n",
        fn_name: "rich_crdt",
        argc: 2,
    },
    // CRDT resolve — a plain field is overwritten (value semantics). Start at a, resolve to b, read b.
    CorpusFn {
        src: "## A Reg is Shared and has:\n    A slot: Int.\n\n## To rich_resolve (a: Int, b: Int) -> Int:\n    Let mutable c be a new Reg with slot a.\n    Resolve c's slot to b.\n    Return c's slot.\n",
        fn_name: "rich_resolve",
        argc: 2,
    },
    // `Append` to a list = a heap push. xs = [a], append b → [a,b]; item 2 + length = b + 2. Proves
    // CrdtAppend ≡ VM's list_push.
    CorpusFn {
        src: "## To rich_append (a: Int, b: Int) -> Int:\n    Let mutable xs be [a].\n    Append b to xs.\n    Return item 2 of xs + length of xs.\n",
        fn_name: "rich_append",
        argc: 2,
    },
    // CRDT merge — a plain-Int `Shared` struct merges per-field ADD. c1(total=a) merge c2(total=b) → a+b.
    CorpusFn {
        src: "## A Counter is Shared and has:\n    A total: Int.\n\n## To rich_merge (a: Int, b: Int) -> Int:\n    Let mutable c1 be a new Counter with total a.\n    Let mutable c2 be a new Counter with total b.\n    Merge c2 into c1.\n    Return c1's total.\n",
        fn_name: "rich_merge",
        argc: 2,
    },
    // `Sleep` — a temporal delay with no data effect. In the clockless fabric it is a vacuous no-op; the
    // differential (via the concurrency fallback, since `Sleep` needs the scheduler driver on the VM) pins
    // `rich_sleep(a,b) = a+b` with two sleeps straddling the arithmetic ≡ the concurrent VM.
    CorpusFn {
        src: "## To rich_sleep (a: Int, b: Int) -> Int:\n    Sleep 50.\n    Let c be a + b.\n    Sleep 10.\n    Return c.\n",
        fn_name: "rich_sleep",
        argc: 2,
    },
    // Short-circuit `or` — `a is 0 or b is 0` materialises a Bool TEMP via a JumpIfTrue/JumpIfFalse diamond,
    // then the real `If` branches on it. Fuzzing a,b over ±2000 rarely hits 0, but the corpus also passes 0
    // via the sign path, and the DIFFERENTIAL (fabric ≡ VM on every drawn pair) pins the if-converted
    // short-circuit exactly. Proves `JumpIfTrue` (the or/and mirror) lowers ≡ VM.
    CorpusFn {
        src: "## To rich_or (a: Int, b: Int) -> Int:\n    Let mutable r be 7.\n    If a is 0 or b is 0:\n        Set r to 1.\n    Return r.\n",
        fn_name: "rich_or",
        argc: 2,
    },
    // Short-circuit `and` — the dual: `a is 0 and b is 0` short-circuits on the FIRST false. Same JumpIfTrue
    // materialisation, opposite polarity; the differential pins it ≡ VM.
    CorpusFn {
        src: "## To rich_and (a: Int, b: Int) -> Int:\n    Let mutable r be 7.\n    If a is 0 and b is 0:\n        Set r to 1.\n    Return r.\n",
        fn_name: "rich_and",
        argc: 2,
    },
    // Three-way `or` — nested short-circuit (`a or b or c`) exercises the RECURSIVE JumpIfTrue if-conversion
    // (a diamond whose arm is itself a diamond). Differential ≡ VM.
    CorpusFn {
        src: "## To rich_or3 (a: Int, b: Int, c: Int) -> Int:\n    Let mutable r be 7.\n    If a is 0 or b is 0 or c is 0:\n        Set r to 1.\n    Return r.\n",
        fn_name: "rich_or3",
        argc: 3,
    },
    // Push to a struct field that holds a LIST — a NESTED collection. box.items=[a,b], push c → [a,b,c];
    // item 3 + length = c + 3. Proves ListPushField (read the field's list ptr, push) ≡ VM.
    CorpusFn {
        src: "## A Box has:\n    A items: Seq of Int.\n\n## To rich_pushfield (a: Int, b: Int, c: Int) -> Int:\n    Let mutable box be a new Box with items [a, b].\n    Push c to box's items.\n    Return item 3 of box's items + length of box's items.\n",
        fn_name: "rich_pushfield",
        argc: 3,
    },
];

/// Run a corpus program's differential: fuzz its scalar args, evaluate the heap-threaded fabric against the
/// VM. Any op the fabric can't lower makes `decision_fabric_from_source` panic loudly (never a silent gap).
pub fn run_corpus_fn(cf: &CorpusFn) {
    let df = decision_fabric_from_source(cf.src, cf.fn_name);
    let mut st = 0xC0_12_05_u64 ^ (cf.fn_name.bytes().map(|c| c as u64).sum::<u64>());
    for _ in 0..150 {
        let args: Vec<i64> = (0..cf.argc)
            .map(|_| (mix(&mut st) % 4000) as i64 - 2000)
            .collect();
        let got = eval_decision_fabric_heap(&df, &args, &mut Vec::new());
        let call = args.iter().map(|a| arg(*a)).collect::<Vec<_>>().join(", ");
        let vmsrc = format!("{}## Main\n    Show {}({}).\n", cf.src, cf.fn_name, call);
        // A channel program can't run on the plain VM (no scheduler); fall back to the scheduler-driving
        // runner — for a sequential (spawn-free) program its output equals the deterministic FIFO order.
        let out = {
            let o = vm_outcome(&vmsrc);
            if o.error.as_deref() == Some("vm: concurrency op requires the scheduler driver") {
                vm_outcome_concurrent(&vmsrc)
            } else {
                o
            }
        };
        assert!(
            out.error.is_none(),
            "{}: VM error {:?}",
            cf.fn_name,
            out.error
        );
        let want: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            got, want,
            "{} fabric != VM on {args:?}: fabric {got}, vm {want}",
            cf.fn_name
        );
    }
}

/// Straight-line whole `Main` programs — the output TAPE covers `Show` and validates the fabric end-to-end
/// (calls inlined, collections on the shared heap). Ints only, so the VM stdout parses to the same tape.
pub const WHOLE_CORPUS: &[&str] = &[
    "## To sq (x: Int) -> Int:\n    Return x * x.\n\n## Main\n    Let a be 5.\n    Show sq(a).\n    Show a + 1.\n    Let xs be [10, 20, 30].\n    Show item 2 of xs.\n    Show length of xs.\n",
    // Text literals rendered as strings, interleaved with an Int — proves the mixed text/int output tape
    // (char-list decode) ≡ VM stdout, line for line. The foundation the FormatValue/Concat ops build on.
    "## Main\n    Show \"hello\".\n    Show 42.\n    Show \"world\".\n",
    // Interpolation (Concat) — exercises the byte-exact int→decimal ACTUAL STRING for zero, a multi-digit
    // value, and a negative: "v=0", "v=12345", "v=-42" ≡ VM stdout.
    "## Main\n    Let a be 0.\n    Show \"v={a}\".\n    Let b be 12345.\n    Show \"v={b}\".\n    Let c be -42.\n    Show \"v={c}\".\n",
    // Promoted GLOBAL — `base` is a Main-scope `Let` read inside `addbase`, so it compiles to a `GlobalSet`
    // (definition) + a `GlobalGet` in the inlined callee. In the straight-line fabric the global is a pure
    // dataflow wire (set-before-use), so the two calls render 1005 and 1020 ≡ VM stdout.
    "## To addbase (n: Int) -> Int:\n    Return n + base.\n## Main\n    Let base be 1000.\n    Show addbase(5).\n    Show addbase(20).\n",
];

/// Whole-program differential: lower `Main` to the fabric, RENDER its output tape (Int→decimal, text→its
/// string), and assert it equals the VM's stdout, line for line. Proves `Show` (int AND text) end-to-end.
pub fn run_whole_fn(src: &str) {
    let wpf = whole_program_fabric_from_source(src);
    let got = eval_whole_program(&wpf);
    let out = vm_outcome(src);
    assert!(
        out.error.is_none(),
        "whole-program VM error: {:?}",
        out.error
    );
    let want: Vec<String> = out.output.lines().map(str::to_string).collect();
    assert_eq!(got, want, "whole-program fabric != VM for:\n{src}");
}
