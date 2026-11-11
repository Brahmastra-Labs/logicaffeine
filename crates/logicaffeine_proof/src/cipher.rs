//! **Reduced-round ciphers as CNF**, for cryptanalysis by SAT. A bit-level [`Circuit`] builder folds constants and
//! shares equal gates, and carries the word operations ARX and stream ciphers are made of: XOR, AND, choice,
//! majority, rotation, and addition modulo 2³² by a ripple-carry adder whose sum bits are three-input XORs (eight
//! clauses each, the parity pattern [`crate::lyapunov::extract_xor`] reads back) and whose carries are majorities.
//!
//! Two families of planted instances stand on it, each made from a known solution and checked by recomputing the
//! cipher directly, never by trusting the encoding:
//!
//! - [`Sha1Preimage`]: a preimage of SHA-1's compression function cut to its first `r` of 80 steps, the digest
//!   fixed and all but some message bits unknown.
//! - [`TriviumRecovery`]: Trivium's 288-bit state recovered from its keystream, some state bits given.
//!
//! [`sha1_compress`] and [`trivium_keystream`] compute the ciphers themselves.

use std::collections::HashMap;

use crate::cdcl::{Lit, Var};
use crate::dimacs::DimacsCnf;

/// A bit of a circuit: a constant, or a literal of its CNF.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bit {
    Const(bool),
    Lit(Lit),
}

impl Bit {
    /// The bit's negation.
    pub fn not(self) -> Bit {
        match self {
            Bit::Const(b) => Bit::Const(!b),
            Bit::Lit(l) => Bit::Lit(l.negated()),
        }
    }

    /// The bit's value under `model`, an assignment of every variable of its circuit.
    pub fn value(self, model: &[bool]) -> bool {
        match self {
            Bit::Const(b) => b,
            Bit::Lit(l) => model[l.var() as usize] == l.is_positive(),
        }
    }
}

/// A 32-bit word of a circuit, least significant bit first.
pub type Word = [Bit; 32];

/// `x` as a word of constants.
pub fn word_const(x: u32) -> Word {
    std::array::from_fn(|i| Bit::Const(x >> i & 1 == 1))
}

/// `w` rotated left by `k` bits.
pub fn rotl(w: &Word, k: usize) -> Word {
    std::array::from_fn(|i| w[(i + 32 - k % 32) % 32])
}

/// The value of `w` under `model`.
pub fn word_value(w: &Word, model: &[bool]) -> u32 {
    w.iter().enumerate().fold(0, |x, (i, b)| x | (b.value(model) as u32) << i)
}

/// A circuit under construction, and its CNF. A gate over constants folds to a constant or to one of its inputs, and
/// a gate met before over the same inputs is the same literal (two inputs of an AND, or the variables of an XOR, its
/// output's sign carrying their signs).
#[derive(Default)]
pub struct Circuit {
    num_vars: usize,
    clauses: Vec<Vec<Lit>>,
    ands: HashMap<(Lit, Lit), Lit>,
    xors: HashMap<Vec<Var>, Lit>,
    majorities: HashMap<[Lit; 3], Lit>,
}

impl Circuit {
    pub fn new() -> Circuit {
        Circuit::default()
    }

    /// The variables so far.
    pub fn num_vars(&self) -> usize {
        self.num_vars
    }

    /// A fresh input bit.
    pub fn input(&mut self) -> Bit {
        Bit::Lit(Lit::pos(self.fresh()))
    }

    /// A word of fresh input bits.
    pub fn word_input(&mut self) -> Word {
        std::array::from_fn(|_| self.input())
    }

    fn fresh(&mut self) -> Var {
        self.num_vars += 1;
        (self.num_vars - 1) as Var
    }

    /// `a ∧ b`.
    pub fn and(&mut self, a: Bit, b: Bit) -> Bit {
        let (a, b) = match (a, b) {
            (Bit::Const(false), _) | (_, Bit::Const(false)) => return Bit::Const(false),
            (Bit::Const(true), x) | (x, Bit::Const(true)) => return x,
            (Bit::Lit(a), Bit::Lit(b)) => (a, b),
        };
        if a == b {
            return Bit::Lit(a);
        }
        if a == b.negated() {
            return Bit::Const(false);
        }
        let key = if a.index() < b.index() { (a, b) } else { (b, a) };
        if let Some(&o) = self.ands.get(&key) {
            return Bit::Lit(o);
        }
        let o = Lit::pos(self.fresh());
        self.clauses.push(vec![o.negated(), a]);
        self.clauses.push(vec![o.negated(), b]);
        self.clauses.push(vec![o, a.negated(), b.negated()]);
        self.ands.insert(key, o);
        Bit::Lit(o)
    }

    /// `a ∨ b`.
    pub fn or(&mut self, a: Bit, b: Bit) -> Bit {
        self.and(a.not(), b.not()).not()
    }

    /// The XOR of `bits`: constants and pairs of one variable folded away, the rest one gate of `2^(k−1)` clauses of
    /// `k + 1` literals over its `k` variables, shared by every XOR of those variables.
    pub fn xor_of(&mut self, bits: &[Bit]) -> Bit {
        let mut parity = false;
        let mut vars: Vec<Var> = Vec::with_capacity(bits.len());
        for b in bits {
            match *b {
                Bit::Const(c) => parity ^= c,
                Bit::Lit(l) => {
                    parity ^= !l.is_positive();
                    vars.push(l.var());
                }
            }
        }
        vars.sort_unstable();
        let mut odd: Vec<Var> = Vec::with_capacity(vars.len());
        for v in vars {
            if odd.last() == Some(&v) {
                odd.pop();
            } else {
                odd.push(v);
            }
        }
        let flip = |b: Bit| if parity { b.not() } else { b };
        match odd.len() {
            0 => return Bit::Const(parity),
            1 => return flip(Bit::Lit(Lit::pos(odd[0]))),
            _ => {}
        }
        if let Some(&o) = self.xors.get(&odd) {
            return flip(Bit::Lit(o));
        }
        let o = self.fresh();
        let all: Vec<Var> = odd.iter().copied().chain([o]).collect();
        for signs in 0..1u32 << all.len() {
            if signs.count_ones() % 2 == 1 {
                self.clauses.push(all.iter().enumerate().map(|(i, &v)| Lit::new(v, signs >> i & 1 == 0)).collect());
            }
        }
        self.xors.insert(odd, Lit::pos(o));
        flip(Bit::Lit(Lit::pos(o)))
    }

    /// `a ⊕ b`.
    pub fn xor(&mut self, a: Bit, b: Bit) -> Bit {
        self.xor_of(&[a, b])
    }

    /// The majority of `a`, `b` and `c`: one gate of six clauses over literals, folded where an input is constant or
    /// two meet.
    pub fn maj(&mut self, a: Bit, b: Bit, c: Bit) -> Bit {
        let lits: Vec<Lit> = match (a, b, c) {
            (Bit::Const(x), y, z) | (y, Bit::Const(x), z) | (y, z, Bit::Const(x)) => {
                return if x { self.or(y, z) } else { self.and(y, z) };
            }
            (Bit::Lit(a), Bit::Lit(b), Bit::Lit(c)) => vec![a, b, c],
        };
        let mut key = [lits[0], lits[1], lits[2]];
        key.sort_unstable_by_key(|l| l.index());
        for (x, y, z) in [(key[0], key[1], key[2]), (key[1], key[2], key[0]), (key[0], key[2], key[1])] {
            if x == y {
                return Bit::Lit(x);
            }
            if x == y.negated() {
                return Bit::Lit(z);
            }
        }
        if let Some(&m) = self.majorities.get(&key) {
            return Bit::Lit(m);
        }
        let m = Lit::pos(self.fresh());
        let [a, b, c] = key;
        for (x, y) in [(a, b), (a, c), (b, c)] {
            self.clauses.push(vec![x.negated(), y.negated(), m]);
            self.clauses.push(vec![x, y, m.negated()]);
        }
        self.majorities.insert(key, m);
        Bit::Lit(m)
    }

    /// SHA-1's choice: `y` where `x` holds, `z` where it does not — `z ⊕ (x ∧ (y ⊕ z))`.
    pub fn ch(&mut self, x: Bit, y: Bit, z: Bit) -> Bit {
        let yz = self.xor(y, z);
        let picked = self.and(x, yz);
        self.xor(z, picked)
    }

    /// `a ⊕ b`, bit by bit.
    pub fn word_xor(&mut self, a: &Word, b: &Word) -> Word {
        std::array::from_fn(|i| self.xor(a[i], b[i]))
    }

    /// `a + b` modulo 2³², by a ripple-carry adder.
    pub fn add(&mut self, a: &Word, b: &Word) -> Word {
        let mut carry = Bit::Const(false);
        std::array::from_fn(|i| {
            let sum = self.xor_of(&[a[i], b[i], carry]);
            carry = self.maj(a[i], b[i], carry);
            sum
        })
    }

    /// Require `b` to read `value`: a unit clause, or the empty clause for a constant that reads otherwise.
    pub fn require(&mut self, b: Bit, value: bool) {
        match b {
            Bit::Const(c) if c == value => {}
            Bit::Const(_) => self.clauses.push(Vec::new()),
            Bit::Lit(l) => self.clauses.push(vec![if value { l } else { l.negated() }]),
        }
    }

    /// The circuit's CNF.
    pub fn into_cnf(self) -> DimacsCnf {
        DimacsCnf { num_vars: self.num_vars, clauses: self.clauses }
    }
}

/// SHA-1's initial chaining value.
pub const SHA1_IV: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];

/// SHA-1's round constants, one per twenty steps.
const SHA1_K: [u32; 4] = [0x5A82_7999, 0x6ED9_EBA1, 0x8F1B_BCDC, 0xCA62_C1D6];

/// SHA-1's compression of `block` into `state`, cut to its first `steps` of 80 steps, the state fed forward.
pub fn sha1_compress(state: [u32; 5], block: &[u32; 16], steps: usize) -> [u32; 5] {
    let mut w = [0u32; 80];
    w[..16].copy_from_slice(block);
    for t in 16..80 {
        w[t] = (w[t - 3] ^ w[t - 8] ^ w[t - 14] ^ w[t - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = state;
    for (t, &wt) in w.iter().enumerate().take(steps) {
        let f = match t / 20 {
            0 => (b & c) | (!b & d),
            2 => (b & c) | (b & d) | (c & d),
            _ => b ^ c ^ d,
        };
        let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(SHA1_K[t / 20]).wrapping_add(wt);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = temp;
    }
    [
        state[0].wrapping_add(a),
        state[1].wrapping_add(b),
        state[2].wrapping_add(c),
        state[3].wrapping_add(d),
        state[4].wrapping_add(e),
    ]
}

/// [`sha1_compress`] as a circuit over words of `circuit`.
pub fn sha1_circuit(circuit: &mut Circuit, state: &[Word; 5], block: &[Word; 16], steps: usize) -> [Word; 5] {
    let mut w: Vec<Word> = block.to_vec();
    for t in 16..steps.max(16) {
        let mixed = [w[t - 3], w[t - 8], w[t - 14], w[t - 16]];
        let x: Word = std::array::from_fn(|i| circuit.xor_of(&[mixed[0][i], mixed[1][i], mixed[2][i], mixed[3][i]]));
        w.push(rotl(&x, 1));
    }
    let [mut a, mut b, mut c, mut d, mut e] = *state;
    for (t, wt) in w.iter().enumerate().take(steps) {
        let f: Word = match t / 20 {
            0 => std::array::from_fn(|i| circuit.ch(b[i], c[i], d[i])),
            2 => std::array::from_fn(|i| circuit.maj(b[i], c[i], d[i])),
            _ => std::array::from_fn(|i| circuit.xor_of(&[b[i], c[i], d[i]])),
        };
        let mut temp = circuit.add(&rotl(&a, 5), &f);
        temp = circuit.add(&temp, &e);
        temp = circuit.add(&temp, &word_const(SHA1_K[t / 20]));
        temp = circuit.add(&temp, wt);
        e = d;
        d = c;
        c = rotl(&b, 30);
        b = a;
        a = temp;
    }
    let out = [a, b, c, d, e];
    std::array::from_fn(|i| circuit.add(&state[i], &out[i]))
}

/// A SplitMix64 stream from `seed`.
fn stream(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// `count` of the positions `0..len`, drawn by `next`, in order.
fn drawn(next: &mut impl FnMut() -> u64, len: usize, count: usize) -> Vec<usize> {
    let mut positions: Vec<usize> = (0..len).collect();
    for i in 0..count.min(len) {
        let j = i + (next() % (len - i) as u64) as usize;
        positions.swap(i, j);
    }
    let mut chosen = positions[..count.min(len)].to_vec();
    chosen.sort_unstable();
    chosen
}

/// A preimage of SHA-1's compression function cut to `steps` steps, from the standard initial value: the digest of
/// a random block fixed, the block's bits unknown but for `known` of them, drawn at random and fixed to the block's.
/// The planted block is a solution; there may be others.
pub struct Sha1Preimage {
    pub cnf: DimacsCnf,
    pub steps: usize,
    pub digest: [u32; 5],
    /// The block, bit `32·i + j` being bit `j` of word `i`, each bit a constant (known) or a literal of `cnf`.
    pub block: Vec<Bit>,
    /// The block the instance was made from.
    pub planted: [u32; 16],
}

impl Sha1Preimage {
    pub fn new(steps: usize, known: usize, seed: u64) -> Sha1Preimage {
        let mut next = stream(seed);
        let planted: [u32; 16] = std::array::from_fn(|_| next() as u32);
        let digest = sha1_compress(SHA1_IV, &planted, steps);
        let given = drawn(&mut next, 512, known);
        let mut circuit = Circuit::new();
        let mut block: Vec<Bit> = Vec::with_capacity(512);
        for k in 0..512 {
            let bit = planted[k / 32] >> (k % 32) & 1 == 1;
            block.push(if given.binary_search(&k).is_ok() { Bit::Const(bit) } else { circuit.input() });
        }
        let words: [Word; 16] = std::array::from_fn(|i| std::array::from_fn(|j| block[32 * i + j]));
        let state = SHA1_IV.map(word_const);
        let out = sha1_circuit(&mut circuit, &state, &words, steps);
        for (word, &want) in out.iter().zip(&digest) {
            for (i, &b) in word.iter().enumerate() {
                circuit.require(b, want >> i & 1 == 1);
            }
        }
        Sha1Preimage { cnf: circuit.into_cnf(), steps, digest, block, planted }
    }

    /// The block `model` assigns.
    pub fn block_of(&self, model: &[bool]) -> [u32; 16] {
        std::array::from_fn(|i| (0..32).fold(0, |x, j| x | (self.block[32 * i + j].value(model) as u32) << j))
    }

    /// Whether `model`'s block is a preimage of the digest, by computing the cut compression function on it.
    pub fn check(&self, model: &[bool]) -> bool {
        sha1_compress(SHA1_IV, &self.block_of(model), self.steps) == self.digest
    }
}

/// One step of Trivium on the 288-bit state `s` (`s[0]` being the specification's `s₁`): its keystream bit
/// `t₁ ⊕ t₂ ⊕ t₃`, the three registers shifted.
fn trivium_step(s: &mut [bool; 288]) -> bool {
    let mut t1 = s[65] ^ s[92];
    let mut t2 = s[161] ^ s[176];
    let mut t3 = s[242] ^ s[287];
    let z = t1 ^ t2 ^ t3;
    t1 ^= (s[90] & s[91]) ^ s[170];
    t2 ^= (s[174] & s[175]) ^ s[263];
    t3 ^= (s[285] & s[286]) ^ s[68];
    s.copy_within(0..92, 1);
    s[0] = t3;
    s.copy_within(93..176, 94);
    s[93] = t1;
    s.copy_within(177..287, 178);
    s[177] = t2;
    z
}

/// `n` bits of Trivium's keystream from the 288-bit state `state`.
pub fn trivium_keystream(state: &[bool; 288], n: usize) -> Vec<bool> {
    let mut s = *state;
    (0..n).map(|_| trivium_step(&mut s)).collect()
}

/// Trivium's key and IV setup: the 80-bit key and IV loaded (bit `i` of each the specification's `Kᵢ₊₁`, `IVᵢ₊₁`),
/// the last three bits set, and 4·288 steps run without output.
pub fn trivium_setup(key: &[bool; 80], iv: &[bool; 80]) -> [bool; 288] {
    let mut s = [false; 288];
    s[..80].copy_from_slice(key);
    s[93..173].copy_from_slice(iv);
    s[285..].fill(true);
    for _ in 0..4 * 288 {
        trivium_step(&mut s);
    }
    s
}

/// Trivium's state recovered from `n` bits of its keystream: a random 288-bit state's keystream fixed, its bits
/// unknown but for `known` of them, drawn at random and fixed to the state's. The planted state is a solution; with
/// a short keystream there may be others.
pub struct TriviumRecovery {
    pub cnf: DimacsCnf,
    pub keystream: Vec<bool>,
    /// The state at the keystream's start, each bit a constant (known) or a literal of `cnf`.
    pub state: Vec<Bit>,
    pub planted: [bool; 288],
}

impl TriviumRecovery {
    pub fn new(n: usize, known: usize, seed: u64) -> TriviumRecovery {
        let mut next = stream(seed);
        let planted: [bool; 288] = std::array::from_fn(|_| next() & 1 == 1);
        let keystream = trivium_keystream(&planted, n);
        let given = drawn(&mut next, 288, known);
        let mut circuit = Circuit::new();
        let state: Vec<Bit> = (0..288)
            .map(|k| if given.binary_search(&k).is_ok() { Bit::Const(planted[k]) } else { circuit.input() })
            .collect();
        let mut s = state.clone();
        for &z in &keystream {
            let t1 = circuit.xor(s[65], s[92]);
            let t2 = circuit.xor(s[161], s[176]);
            let t3 = circuit.xor(s[242], s[287]);
            let out = circuit.xor_of(&[t1, t2, t3]);
            circuit.require(out, z);
            let a1 = circuit.and(s[90], s[91]);
            let n1 = circuit.xor_of(&[t1, a1, s[170]]);
            let a2 = circuit.and(s[174], s[175]);
            let n2 = circuit.xor_of(&[t2, a2, s[263]]);
            let a3 = circuit.and(s[285], s[286]);
            let n3 = circuit.xor_of(&[t3, a3, s[68]]);
            s.copy_within(0..92, 1);
            s[0] = n3;
            s.copy_within(93..176, 94);
            s[93] = n1;
            s.copy_within(177..287, 178);
            s[177] = n2;
        }
        TriviumRecovery { cnf: circuit.into_cnf(), keystream, state, planted }
    }

    /// The state `model` assigns.
    pub fn state_of(&self, model: &[bool]) -> [bool; 288] {
        std::array::from_fn(|k| self.state[k].value(model))
    }

    /// Whether `model`'s state yields the keystream, by running Trivium on it.
    pub fn check(&self, model: &[bool]) -> bool {
        trivium_keystream(&self.state_of(model), self.keystream.len()) == self.keystream
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/cipher/tests.rs"]
mod tests;
