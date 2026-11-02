use super::*;
use crate::vm::{Op, Vm};

/// Compile `src` the way the production run path does — WITH the resolved type registry
/// (`compile_with_types`, exactly as `run_to_outcome`), so type-directed lowering (struct field
/// access, map value typing, …) produces the real bytecode. The oracle (bounds-check elision) is
/// a perf-only pass and is omitted; it never changes output. Hands the program + interner to `f`.
fn with_compiled<R>(src: &str, f: impl FnOnce(&CompiledProgram, &Interner) -> R) -> R {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("source parses");
        let program = crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
            .expect("source compiles");
        f(&program, interner)
    })
}

fn run(program: &CompiledProgram) -> String {
    let mut vm = Vm::new(program);
    vm.run().expect("decoded program runs");
    vm.output().to_string()
}

// ---- Part 1: the chip opcode map is stable + total ----

/// Every op in the language's instruction catalog has a registered opcode, and every
/// registered name is a real op — the ISA table is a total bijection with the `Op` set.
/// A newly-added `Op` that forgets to register here fails this loud (in CI, before silicon).
#[test]
fn opcode_map_is_total_over_the_isa() {
    let names: Vec<&'static str> = Op::all_variants().iter().map(|op| op.name()).collect();
    // Every op maps to a distinct opcode.
    for op in Op::all_variants() {
        assert!(
            opcode_of_name(op.name()).is_some(),
            "op {} has no opcode — register it in ISA_OPCODES",
            op.name()
        );
    }
    // No stale entries: every table name is a real op.
    for &n in ISA_OPCODES {
        assert!(
            names.contains(&n),
            "ISA_OPCODES has stale entry {n} (no such op)"
        );
    }
    // Sizes agree ⇒ bijection.
    assert_eq!(
        ISA_OPCODES.len(),
        names.len(),
        "ISA table and op catalog disagree in size"
    );
    // Opcodes are unique (no two ops share a number).
    let mut codes: Vec<u16> = Op::all_variants().iter().map(opcode).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), names.len(), "two ops collide on one opcode");
}

/// The reversible/adiabatic core sits at pinned low opcodes — a fabricated decoder wires
/// these constants, so they must never drift. This is the append-only ABI guard.
#[test]
fn reversible_core_opcodes_are_pinned() {
    assert_eq!(opcode_of_name("RevAdd"), Some(0));
    assert_eq!(opcode_of_name("RevSub"), Some(1));
    assert_eq!(opcode_of_name("RevXor"), Some(2));
    assert_eq!(opcode_of_name("RevNeg"), Some(3));
    assert_eq!(opcode_of_name("RevSwap"), Some(4));
}

// ---- Part 2: bytecode travels the wire and executes ----

/// THE flagship: an arithmetic program, encoded to bytes, decoded against a BRAND-NEW
/// interner (a fresh "receiver"), executes byte-identically to the original. This is
/// literally "send the language bytecode over the wire and have it executed."
#[test]
fn bytecode_travels_the_wire_and_executes_in_a_fresh_interner() {
    let src = "## Main\n    Let x be 20.\n    Let y be 22.\n    Show x + y.\n";
    let want = crate::compile::vm_outcome(src).output;
    assert_eq!(want, "42\n");

    let bytes = with_compiled(src, |program, interner| encode_program(program, interner));

    // A fresh interner on the far side — nothing shared with the sender.
    let mut receiver = Interner::new();
    let decoded = decode_program(&bytes, &mut receiver).expect("frame decodes");
    assert_eq!(run(&decoded), want);
}

/// A RECURSIVE, multi-function program (exercises the function table, `fn_index`, `Call`,
/// control flow, `Return`) travels the wire and runs identically through a fresh interner.
/// The source is copied verbatim from a locked machine-crate test, so any failure is the
/// codec's, never the program's.
#[test]
fn a_recursive_program_round_trips_and_runs_identically() {
    let src = "## To sumdown (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Let rest be sumdown(n - 1).\n    Return n + rest.\n## Main\n    Let r be sumdown(3).\n    Show r.\n";
    let want = crate::compile::vm_outcome(src).output;
    assert_eq!(want, "6\n", "oracle: 3+2+1+0 = 6");

    let bytes = with_compiled(src, |program, interner| encode_program(program, interner));
    let mut receiver = Interner::new();
    let decoded = decode_program(&bytes, &mut receiver).expect("frame decodes");
    assert_eq!(run(&decoded), want);
}

/// A collections program (list literal, `Push`, `length of` — exercises `NewList`/`ListPush`/
/// `Length` and the constant pool) travels the wire and runs identically. Source verbatim from
/// a locked debug-crate example.
#[test]
fn a_collections_program_round_trips_and_runs_identically() {
    let src = "## Main\nLet xs be [1, 2, 3].\nPush 4 to xs.\nShow length of xs.\n";
    let want = crate::compile::vm_outcome(src).output;
    assert_eq!(want, "4\n", "oracle: [1,2,3] + 4 has length 4");

    let bytes = with_compiled(src, |program, interner| encode_program(program, interner));
    let mut receiver = Interner::new();
    let decoded = decode_program(&bytes, &mut receiver).expect("frame decodes");
    assert_eq!(run(&decoded), want);
}

/// End-to-end over a REAL external byte boundary: encode → write the bytes to disk (a stand-in
/// for the wire / a device's instruction memory) → read them back on the "far side" with a fresh
/// interner → execute byte-identically. This is the literal "send the bytecode over the wire and
/// have it executed" round-trip, through actual serialized bytes leaving the process's memory.
#[test]
fn bytecode_survives_a_real_byte_boundary_and_executes() {
    let src = "## Main\n    Let a be 100.\n    Let b be 23.\n    Show a - b.\n";
    let want = crate::compile::vm_outcome(src).output;
    assert_eq!(want, "77\n");

    let bytes = with_compiled(src, |program, interner| encode_program(program, interner));

    // Cross a genuine byte boundary: write the frame out, read it back as raw bytes.
    let path = std::env::temp_dir().join(format!("logos_binisa_{}.lgb", std::process::id()));
    std::fs::write(&path, &bytes).expect("frame written to disk");
    let received = std::fs::read(&path).expect("frame read back from disk");
    std::fs::remove_file(&path).ok();
    assert_eq!(received, bytes, "the bytes survived the boundary intact");

    // Decode + run on the far side with nothing shared but the bytes themselves.
    let mut receiver = Interner::new();
    let decoded = decode_program(&received, &mut receiver).expect("received frame decodes");
    assert_eq!(run(&decoded), want);
}

/// A broad corpus spanning EVERY op category — arithmetic, floats, unicode text, booleans,
/// comparisons, all three loop forms, recursion, lists (push/length/index), closures (captures),
/// structs (`struct_types` + field access), enums (`enum_types` + `Inspect`/`When`), maps,
/// multi-function programs (`fn_index`), and sequence concat. Each program, compiled and run on
/// the plain VM, is compared against the SAME program encoded → decoded in a fresh interner →
/// run. Any divergence is a real codec bug (a dropped/mis-remapped field), caught RED here.
fn corpus() -> &'static [(&'static str, &'static str)] {
    &[
        ("arithmetic", "## Main\n    Show 2 + 3 * 4 - 1.\n"),
        ("float", "## Main\n    Let x be 3.14.\n    Let y be 2.0.\n    Show x + y.\n"),
        ("text_unicode", "## Main\n    Show \"héllo wörld ✓ 日本語\".\n"),
        (
            "boolean_if",
            "## Main\n    Let b be true.\n    If b:\n        Show \"yes\".\n    Otherwise:\n        Show \"no\".\n",
        ),
        (
            "comparison",
            "## Main\n    Let x be 3.\n    If x is greater than 5:\n        Show \"big\".\n    Otherwise:\n        Show \"small\".\n",
        ),
        (
            "while",
            "## Main\n    Let mutable i be 0.\n    Let mutable s be 0.\n    While i is less than 5:\n        Set s to s + i.\n        Set i to i + 1.\n    Show s.\n",
        ),
        (
            "repeat_range",
            "## Main\n    Let mutable total be 0.\n    Repeat for i from 1 to 5:\n        Set total to total + i.\n    Show total.\n",
        ),
        (
            "for_in_list",
            "## Main\n    Let mutable sum be 0.\n    Repeat for x in [10, 20, 30]:\n        Set sum to sum + x.\n    Show sum.\n",
        ),
        (
            "recursion_fib",
            "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(6).",
        ),
        ("list_push_len", "## Main\nLet xs be [1, 2, 3].\nPush 4 to xs.\nShow length of xs."),
        ("list_index", "## Main\nLet xs be [10, 20, 30].\nShow item 2 of xs."),
        ("closure_identity", "## Main\nLet f be (x: Int) -> x.\nShow f(42)."),
        ("closure_arith", "## Main\nLet doubler be (n: Int) -> n * 2.\nShow doubler(5)."),
        ("closure_two", "## Main\nLet add be (a: Int, b: Int) -> a + b.\nShow add(3, 7)."),
        (
            "struct",
            "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point.\nShow p's x.",
        ),
        (
            "enum_when",
            "## A Color is one of:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Green.\nInspect c:\n    When Red: Show \"red\".\n    When Green: Show \"green\".\n    When Blue: Show \"blue\".\n",
        ),
        (
            "map",
            "## Main\nLet mut prices be a new Map of Text to Int.\nSet item \"iron\" of prices to 100.\nLet cost be item \"iron\" of prices.\nShow cost.",
        ),
        (
            "multi_function",
            "## To blend (a: Int, b: Int) -> Int:\n    Return (a * b) + (a + b).\n## Main\n    Let x be 5.\n    Let y be blend(x, 3).\n    Show y.\n",
        ),
    ]
}

#[test]
fn the_op_catalog_round_trips_across_a_broad_corpus() {
    for (name, src) in corpus() {
        // Compile once; capture the frame AND the original plain-VM output.
        let (bytes, want) = with_compiled(src, |p, i| {
            let mut vm = Vm::new(p);
            vm.run()
                .unwrap_or_else(|e| panic!("{name}: original failed to run: {e}"));
            (encode_program(p, i), vm.output().to_string())
        });
        assert!(
            !want.is_empty(),
            "{name}: original produced no output (corpus program invalid)"
        );

        let mut receiver = Interner::new();
        let decoded = decode_program(&bytes, &mut receiver)
            .unwrap_or_else(|| panic!("{name}: frame failed to decode"));
        let mut vm2 = Vm::new(&decoded);
        vm2.run()
            .unwrap_or_else(|e| panic!("{name}: decoded failed to run: {e}"));
        assert_eq!(
            vm2.output(),
            want,
            "{name}: wire round-trip diverged from the original"
        );
    }
}

/// Decoding is LOSSLESS: re-encoding a decoded program reproduces the original bytes exactly, so
/// the encoding is a fixed point — `encode ∘ decode ∘ encode == encode`. This proves the decoder
/// drops NOTHING the encoder captured (a silently-dropped field would perturb `bytes2`), across
/// the whole corpus. It also re-confirms determinism now that `fn_index` is order-normalized.
#[test]
fn decoding_is_lossless_a_fixed_point_of_encoding() {
    for (name, src) in corpus() {
        let bytes1 = with_compiled(src, |p, i| encode_program(p, i));
        let mut receiver = Interner::new();
        let decoded =
            decode_program(&bytes1, &mut receiver).unwrap_or_else(|| panic!("{name}: decodes"));
        let bytes2 = encode_program(&decoded, &receiver);
        assert_eq!(
            bytes1, bytes2,
            "{name}: decode is not a fixed point of encode — a field was dropped or reordered"
        );
    }
}

/// The frame is byte-for-byte deterministic — the same program encodes to the same bytes
/// every time (a stable artifact you can hash, cache, and ship reproducibly to a device).
#[test]
fn encoding_is_deterministic() {
    let src = "## Main\n    Show 6 * 7.\n";
    let a = with_compiled(src, |p, i| encode_program(p, i));
    let b = with_compiled(src, |p, i| encode_program(p, i));
    assert_eq!(a, b);
}

/// `encoding_is_deterministic`'s own two-shot check is not enough headroom to catch a `HashMap`-
/// order hazard: two encodes can coincidentally land the same order more than half the time (there
/// are only a few entries to permute), so a regression here can hide behind a lucky pair for a long
/// while. Twenty independent compiles of the identical source, twenty independent encodes, one
/// shared reference — every one of them must match it byte for byte, or the encoder is reading
/// something (a struct/enum type table, a loop-locals map, …) straight off a `HashMap`'s iteration
/// order instead of a canonical one.
#[test]
fn encoding_is_deterministic_across_twenty_independent_compiles() {
    let src = "## A File has:\n    a name, which is Text.\n    a size, which is Int.\n\n\
               ## An HttpError has:\n    a status, which is Int.\n    a message, which is Text.\n\n\
               ## Main\n\
               Let f be a new File with name \"x\" and size 1.\n\
               Let e be a new HttpError with status 404 and message \"nope\".\n\
               Show f's name.\n\
               Show e's status.\n";
    let reference = with_compiled(src, |p, i| encode_program(p, i));
    for attempt in 0..20 {
        let encoded = with_compiled(src, |p, i| encode_program(p, i));
        assert_eq!(
            encoded, reference,
            "encode #{attempt} diverged from the reference encoding — the encoder is not \
             canonicalizing some table it walks"
        );
    }
}

/// A frame self-identifies (magic) and rejects corruption: flipping any payload byte, a
/// bad magic, or a wrong version all decode to `None` — a corrupt program never executes.
#[test]
fn frame_self_identifies_and_rejects_corruption() {
    let src = "## Main\n    Show 1 + 1.\n";
    let good = with_compiled(src, |p, i| encode_program(p, i));
    assert_eq!(&good[0..4], ISA_MAGIC, "frame opens with the ISA magic");

    // Sanity: the pristine frame decodes and runs.
    let mut i0 = Interner::new();
    assert!(decode_program(&good, &mut i0).is_some());

    // Bad magic.
    let mut bad_magic = good.clone();
    bad_magic[0] = b'X';
    let mut i1 = Interner::new();
    assert!(
        decode_program(&bad_magic, &mut i1).is_none(),
        "bad magic rejected"
    );

    // Wrong version.
    let mut bad_ver = good.clone();
    bad_ver[4] = bad_ver[4].wrapping_add(1);
    let mut i2 = Interner::new();
    assert!(
        decode_program(&bad_ver, &mut i2).is_none(),
        "wrong version rejected"
    );

    // Flipped payload byte (past the 16-byte header) — checksum catches it.
    let mut bad_payload = good.clone();
    let last = bad_payload.len() - 1;
    bad_payload[last] ^= 0xFF;
    let mut i3 = Interner::new();
    assert!(
        decode_program(&bad_payload, &mut i3).is_none(),
        "corrupt payload rejected"
    );

    // Truncated frame.
    let mut i4 = Interner::new();
    assert!(
        decode_program(&good[..8], &mut i4).is_none(),
        "truncated frame rejected"
    );
}

/// A CheckPolicy program (the one op that carries a `Symbol`) survives the wire: its policy
/// predicate is lowered into the string table and re-interned on the far side, so the
/// decoded op names the same predicate. Proves the symbol-remap surface is complete.
#[test]
fn the_symbol_carrying_op_round_trips_its_predicate() {
    // CheckPolicy is the ONE op carrying a `Symbol`. Build a sender interner where the
    // predicate has a NON-TRIVIAL id (several names interned before it), so a fresh receiver
    // interner — which assigns different ids — genuinely exercises the symbol remap.
    let mut sender = Interner::new();
    sender.intern("foo");
    sender.intern("bar");
    let pred = sender.intern("readFile");
    assert_ne!(pred.index(), 0, "predicate is not the trivial empty symbol");

    let mut p = CompiledProgram::default();
    p.code = vec![
        Op::CheckPolicy {
            subject: 0,
            predicate: pred,
            is_capability: false,
            object: u16::MAX,
            source_text: 0,
        },
        Op::Halt,
    ];

    let bytes = encode_program(&p, &sender);
    let mut receiver = Interner::new(); // different id space than the sender
    let decoded = decode_program(&bytes, &mut receiver).expect("decodes");
    match &decoded.code[0] {
        Op::CheckPolicy { predicate, .. } => {
            assert_eq!(
                receiver.resolve(*predicate),
                "readFile",
                "predicate NAME preserved across the wire (ids differ between interners)"
            );
        }
        other => panic!("expected CheckPolicy first, got {:?}", other.name()),
    }
}
