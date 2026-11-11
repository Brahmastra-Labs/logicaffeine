use super::*;
use crate::cdcl::{SolveResult, Solver};

fn rng(seed: u64) -> impl FnMut() -> u64 {
    stream(seed)
}

/// A model of `cnf` by the CDCL solver, if it has one.
fn solve(cnf: &DimacsCnf) -> Option<Vec<bool>> {
    let mut solver = Solver::new(cnf.num_vars);
    for c in &cnf.clauses {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => Some(model),
        SolveResult::Unsat => None,
    }
}

/// SHA-1's one padded block for a message of under 56 bytes.
fn padded(message: &[u8]) -> [u32; 16] {
    let mut bytes = [0u8; 64];
    bytes[..message.len()].copy_from_slice(message);
    bytes[message.len()] = 0x80;
    bytes[56..].copy_from_slice(&(8 * message.len() as u64).to_be_bytes());
    std::array::from_fn(|i| u32::from_be_bytes(bytes[4 * i..4 * i + 4].try_into().expect("four bytes")))
}

#[test]
fn sha1_compress_at_eighty_steps_is_sha1() {
    // FIPS 180's examples: "abc", and the empty message.
    assert_eq!(sha1_compress(SHA1_IV, &padded(b"abc"), 80), [0xA999_3E36, 0x4706_816A, 0xBA3E_2571, 0x7850_C26C, 0x9CD0_D89D]);
    assert_eq!(sha1_compress(SHA1_IV, &padded(b""), 80), [0xDA39_A3EE, 0x5E6B_4B0D, 0x3255_BFEF, 0x9560_1890, 0xAFD8_0709]);
}

#[test]
fn trivium_from_its_key_and_iv_gives_its_published_keystream() {
    // The eSTREAM vector of the key 80 00 … 00 and the IV 00 … 00: Kᵢ runs over the key's bytes from the last, each
    // from its most significant bit, and the keystream fills bytes from their least significant bit.
    let bits = |bytes: [u8; 10]| -> [bool; 80] { std::array::from_fn(|i| bytes[9 - i / 8] >> (7 - i % 8) & 1 == 1) };
    let mut key = [0u8; 10];
    key[0] = 0x80;
    let state = trivium_setup(&bits(key), &bits([0; 10]));
    let z = trivium_keystream(&state, 128);
    let bytes: Vec<u8> = z.chunks(8).map(|b| b.iter().enumerate().fold(0, |x, (i, &bit)| x | (bit as u8) << i)).collect();
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    assert_eq!(hex, "38EB86FF730D7A9CAF8DF13A4420540D");
}

#[test]
fn every_gate_holds_exactly_on_its_truth_table() {
    // With its inputs fixed, a gate's clauses have one model, and its output there is the gate's function of them.
    let gates: Vec<(&str, usize, fn(&mut Circuit, &[Bit]) -> Bit, fn(&[bool]) -> bool)> = vec![
        ("and", 2, |c, x| c.and(x[0], x[1]), |v| v[0] & v[1]),
        ("or", 2, |c, x| c.or(x[0], x[1]), |v| v[0] | v[1]),
        ("xor", 2, |c, x| c.xor(x[0], x[1]), |v| v[0] ^ v[1]),
        ("xor3", 3, |c, x| c.xor_of(&[x[0], x[1], x[2]]), |v| v[0] ^ v[1] ^ v[2]),
        ("xor4", 4, |c, x| c.xor_of(&[x[0], x[1], x[2], x[3]]), |v| v[0] ^ v[1] ^ v[2] ^ v[3]),
        ("maj", 3, |c, x| c.maj(x[0], x[1], x[2]), |v| (v[0] as u8 + v[1] as u8 + v[2] as u8) >= 2),
        ("ch", 3, |c, x| c.ch(x[0], x[1], x[2]), |v| if v[0] { v[1] } else { v[2] }),
    ];
    for (name, arity, build, truth) in gates {
        for negated in 0..1u32 << arity {
            for values in 0..1u32 << arity {
                let mut circuit = Circuit::new();
                let inputs: Vec<Bit> = (0..arity)
                    .map(|i| {
                        let b = circuit.input();
                        if negated >> i & 1 == 1 {
                            b.not()
                        } else {
                            b
                        }
                    })
                    .collect();
                let out = build(&mut circuit, &inputs);
                let read: Vec<bool> = (0..arity).map(|i| values >> i & 1 == 1).collect();
                for (b, &v) in inputs.iter().zip(&read) {
                    circuit.require(*b, v);
                }
                let cnf = circuit.into_cnf();
                let models: Vec<Vec<bool>> = (0..1u32 << cnf.num_vars)
                    .map(|bits| (0..cnf.num_vars).map(|v| bits >> v & 1 == 1).collect::<Vec<bool>>())
                    .filter(|m| cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())))
                    .collect();
                assert_eq!(models.len(), 1, "{name} over {read:?} (negated {negated:b}): one model");
                assert_eq!(out.value(&models[0]), truth(&read), "{name} over {read:?} (negated {negated:b})");
            }
        }
    }
}

#[test]
fn gates_fold_constants_and_share_equal_inputs() {
    let mut c = Circuit::new();
    let (x, y, z) = (c.input(), c.input(), c.input());
    let (t, f) = (Bit::Const(true), Bit::Const(false));
    assert_eq!((c.and(x, f), c.and(x, t), c.and(x, x), c.and(x, x.not())), (f, x, x, f));
    assert_eq!((c.xor(x, f), c.xor(x, t), c.xor(x, x), c.xor(x, x.not())), (x, x.not(), f, t));
    assert_eq!((c.maj(x, y, f), c.maj(x, x, y), c.maj(x, x.not(), z)), (c.and(x, y), x, z));
    assert_eq!(c.maj(x, y, t), c.or(x, y));
    let gates = c.num_vars();
    assert_eq!((c.and(x, y), c.and(y, x)), (c.and(x, y), c.and(x, y)));
    assert_eq!(c.xor(x.not(), y), c.xor(x, y).not(), "an XOR's output carries its inputs' signs");
    assert_eq!(c.xor_of(&[z, y, x]), c.xor_of(&[x, y, z]));
    assert_eq!(c.xor_of(&[x, y, x]), y);
    let shared = c.num_vars();
    assert!(shared <= gates + 2, "equal gates are one: {gates} then {shared} variables");
}

#[test]
fn the_compression_circuit_computes_the_compression_function() {
    // On constant inputs the circuit folds to the digest itself; on unknown inputs fixed by units, the one model's
    // digest is the reference's, at every cut.
    let mut next = rng(0x5A1);
    for steps in [1usize, 15, 16, 17, 20, 21, 40, 60, 80] {
        let block: [u32; 16] = std::array::from_fn(|_| next() as u32);
        let state: [u32; 5] = std::array::from_fn(|_| next() as u32);
        let want = sha1_compress(state, &block, steps);
        let mut folded = Circuit::new();
        let out = sha1_circuit(&mut folded, &state.map(word_const), &block.map(word_const), steps);
        assert_eq!(folded.num_vars(), 0, "steps {steps}: constants fold");
        assert_eq!(out.map(|w| word_value(&w, &[])), want, "steps {steps}");
        let mut circuit = Circuit::new();
        let words: [Word; 16] = std::array::from_fn(|_| circuit.word_input());
        let out = sha1_circuit(&mut circuit, &state.map(word_const), &words, steps);
        for (w, &x) in words.iter().zip(&block) {
            for (i, &b) in w.iter().enumerate() {
                circuit.require(b, x >> i & 1 == 1);
            }
        }
        let model = solve(&circuit.into_cnf()).expect("the inputs fixed, the circuit has a model");
        assert_eq!(out.map(|w| word_value(&w, &model)), want, "steps {steps}");
    }
}

#[test]
fn a_planted_preimage_is_a_model_and_every_model_found_is_a_preimage() {
    // The planted block satisfies its instance, and the CDCL solver's model of each is checked by computing the cut
    // compression function on its block: the cuts the search takes in under a second, and the whole function with
    // all but four of the block's bits given.
    for (steps, known, seed) in [(8usize, 0usize, 0u64), (16, 0, 0), (17, 0, 0), (18, 0, 0), (18, 128, 0), (80, 508, 0)] {
        let instance = Sha1Preimage::new(steps, known, seed);
        let unknown = instance.block.iter().filter(|b| matches!(b, Bit::Lit(_))).count();
        assert_eq!(unknown, 512 - known, "steps {steps}: the unknown bits");
        let model = solve(&instance.cnf).expect("the planted block is a preimage");
        assert!(instance.check(&model), "steps {steps}: the model's block is a preimage");
    }
}

#[test]
fn a_planted_state_is_recovered_from_its_keystream() {
    for (n, known, seed) in [(64usize, 240usize, 1u64), (200, 200, 2), (300, 180, 3)] {
        let instance = TriviumRecovery::new(n, known, seed);
        assert_eq!(instance.keystream, trivium_keystream(&instance.planted, n));
        let unknown = instance.state.iter().filter(|b| matches!(b, Bit::Lit(_))).count();
        assert_eq!(unknown, 288 - known);
        let model = solve(&instance.cnf).expect("the planted state yields the keystream");
        assert!(instance.check(&model), "n {n}: the model's state yields the keystream");
    }
}

#[test]
fn a_wrong_digest_or_keystream_is_refused_by_the_check() {
    let instance = Sha1Preimage::new(16, 400, 7);
    let model = solve(&instance.cnf).expect("a preimage");
    let mut other = Sha1Preimage::new(16, 400, 7);
    other.digest[2] ^= 1;
    assert!(instance.check(&model) && !other.check(&model));
    let stream = TriviumRecovery::new(100, 220, 8);
    let model = solve(&stream.cnf).expect("a state");
    let mut other = TriviumRecovery::new(100, 220, 8);
    other.keystream[50] = !other.keystream[50];
    assert!(stream.check(&model) && !other.check(&model));
}
