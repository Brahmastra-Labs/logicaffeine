//! The multi-word integer core: every wider-than-a-word integer operation, built once from lane operations
//! and parameterized by the lane shape.
//!
//! A [`Builder`] writes a [`Routine`]; its word-level methods emit one lane operation each, and its `mw_*`
//! methods compose them into operations on multi-word values ([`Mw`], little-endian words). The algorithms are
//! the textbook ones a narrow datapath uses: ripple carries, schoolbook products from multiplier-width digits
//! accumulated by column, restoring division, barrel shifts from constant shifts and selects, a binary-search
//! leading-zero count. None branches on data.

use std::collections::BTreeMap;

use super::lane::{LInst, LOp, LReg, LStmt, LaneShape, Routine};

/// A multi-word value: little-endian lane registers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mw(pub Vec<LReg>);

impl Mw {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The words `from..to` (clipped), as a value.
    pub fn slice(&self, from: usize, to: usize) -> Mw {
        Mw(self.0[from.min(self.len())..to.min(self.len())].to_vec())
    }
}

/// The routine under construction.
pub struct Builder {
    shape: LaneShape,
    regs: u32,
    consts: BTreeMap<u32, LReg>,
    prelude: Vec<LStmt>,
    bodies: Vec<Vec<LStmt>>,
}

impl Builder {
    pub fn new(shape: LaneShape) -> Builder {
        Builder {
            shape,
            regs: 0,
            consts: BTreeMap::new(),
            prelude: Vec::new(),
            bodies: vec![Vec::new()],
        }
    }

    pub fn shape(&self) -> LaneShape {
        self.shape
    }

    /// The bits of one word.
    pub fn w(&self) -> u32 {
        self.shape.word
    }

    /// A register no instruction has written yet.
    pub fn fresh(&mut self) -> LReg {
        let r = self.regs;
        self.regs += 1;
        r
    }

    fn push(&mut self, inst: LInst) {
        self.bodies
            .last_mut()
            .expect("a body under construction")
            .push(LStmt::Inst(inst));
    }

    fn emit(&mut self, op: LOp) -> LReg {
        let dst = self.fresh();
        self.push(LInst {
            dst,
            dst2: None,
            op,
        });
        dst
    }

    fn emit2(&mut self, op: LOp) -> (LReg, LReg) {
        let dst = self.fresh();
        let dst2 = self.fresh();
        self.push(LInst {
            dst,
            dst2: Some(dst2),
            op,
        });
        (dst, dst2)
    }

    /// The constant `v`, materialized once in the routine's prelude.
    pub fn k(&mut self, v: u32) -> LReg {
        assert!(
            v & !self.shape.mask() == 0,
            "constant {v:#x} is wider than a {}-bit word",
            self.shape.word
        );
        if let Some(r) = self.consts.get(&v) {
            return *r;
        }
        let dst = self.fresh();
        self.prelude.push(LStmt::Inst(LInst {
            dst,
            dst2: None,
            op: LOp::Const(v),
        }));
        self.consts.insert(v, dst);
        dst
    }

    /// `dst = src`, for a register carried across the trips of a `repeat`.
    pub fn assign(&mut self, dst: LReg, src: LReg) {
        if dst != src {
            self.push(LInst {
                dst,
                dst2: None,
                op: LOp::Mov(src),
            });
        }
    }

    /// Run `f` as a body repeated `times` times.
    pub fn repeat(&mut self, times: u32, f: impl FnOnce(&mut Builder)) {
        self.bodies.push(Vec::new());
        f(self);
        let body = self.bodies.pop().expect("the repeat body");
        self.bodies
            .last_mut()
            .expect("the enclosing body")
            .push(LStmt::Repeat { times, body });
    }

    /// The finished routine.
    pub fn finish(
        mut self,
        name: impl Into<String>,
        inputs: Vec<LReg>,
        outputs: Vec<LReg>,
    ) -> Routine {
        let body = self.bodies.pop().expect("the routine body");
        assert!(self.bodies.is_empty(), "every repeat closed");
        let mut all = std::mem::take(&mut self.prelude);
        all.extend(body);
        Routine {
            name: name.into(),
            shape: self.shape,
            regs: self.regs,
            inputs,
            outputs,
            body: all,
        }
    }

    // ── Words ──────────────────────────────────────────────────────────────────────────────────────────

    pub fn add(&mut self, a: LReg, b: LReg, cin: LReg) -> (LReg, LReg) {
        self.emit2(LOp::Add { a, b, cin })
    }

    pub fn sub(&mut self, a: LReg, b: LReg, bin: LReg) -> (LReg, LReg) {
        self.emit2(LOp::Sub { a, b, bin })
    }

    pub fn mul(&mut self, a: LReg, b: LReg) -> LReg {
        self.emit(LOp::Mul { a, b })
    }

    pub fn and(&mut self, a: LReg, b: LReg) -> LReg {
        self.emit(LOp::And { a, b })
    }

    pub fn or(&mut self, a: LReg, b: LReg) -> LReg {
        self.emit(LOp::Or { a, b })
    }

    pub fn xor(&mut self, a: LReg, b: LReg) -> LReg {
        self.emit(LOp::Xor { a, b })
    }

    pub fn not(&mut self, a: LReg) -> LReg {
        self.emit(LOp::Not { a })
    }

    pub fn shl(&mut self, a: LReg, k: u32) -> LReg {
        if k == 0 {
            return a;
        }
        assert!(k < self.w());
        self.emit(LOp::Shl { a, k })
    }

    pub fn shr(&mut self, a: LReg, k: u32) -> LReg {
        if k == 0 {
            return a;
        }
        assert!(k < self.w());
        self.emit(LOp::Shr { a, k })
    }

    pub fn ltu(&mut self, a: LReg, b: LReg) -> LReg {
        self.emit(LOp::Ltu { a, b })
    }

    pub fn eq(&mut self, a: LReg, b: LReg) -> LReg {
        self.emit(LOp::Eq { a, b })
    }

    pub fn sel(&mut self, c: LReg, t: LReg, f: LReg) -> LReg {
        if t == f {
            return t;
        }
        self.emit(LOp::Sel { c, t, f })
    }

    /// `1 − b` for a bit `b`.
    pub fn bit_not(&mut self, b: LReg) -> LReg {
        let one = self.k(1);
        self.xor(b, one)
    }

    /// Bit `i` of a word, as 0 or 1.
    pub fn word_bit(&mut self, a: LReg, i: u32) -> LReg {
        let s = self.shr(a, i);
        let one = self.k(1);
        self.and(s, one)
    }

    /// 1 when the word is nonzero.
    pub fn nonzero(&mut self, a: LReg) -> LReg {
        let z = self.k(0);
        let e = self.eq(a, z);
        self.bit_not(e)
    }

    // ── Multi-word values ──────────────────────────────────────────────────────────────────────────────

    /// `n` fresh registers, for a routine's inputs.
    pub fn mw_input(&mut self, n: usize) -> Mw {
        Mw((0..n).map(|_| self.fresh()).collect())
    }

    /// The constant `v` in `n` words.
    pub fn mw_const(&mut self, v: u128, n: usize) -> Mw {
        let w = self.w();
        let mask = self.shape.mask() as u128;
        Mw((0..n)
            .map(|i| {
                let at = i as u32 * w;
                let word = if at >= 128 {
                    0
                } else {
                    ((v >> at) & mask) as u32
                };
                self.k(word)
            })
            .collect())
    }

    pub fn mw_zero(&mut self, n: usize) -> Mw {
        self.mw_const(0, n)
    }

    /// Fresh registers holding a copy of `a`: the state a `repeat` carries.
    pub fn mw_state(&mut self, a: &Mw) -> Mw {
        let regs: Vec<LReg> = a.0.iter().map(|_| self.fresh()).collect();
        for (d, s) in regs.iter().zip(&a.0) {
            self.push(LInst {
                dst: *d,
                dst2: None,
                op: LOp::Mov(*s),
            });
        }
        Mw(regs)
    }

    /// A fresh register holding a copy of `a`.
    pub fn state(&mut self, a: LReg) -> LReg {
        let r = self.fresh();
        self.push(LInst {
            dst: r,
            dst2: None,
            op: LOp::Mov(a),
        });
        r
    }

    /// `dst = src` as one parallel assignment: every word of `src` is read before any word of `dst` is
    /// written. A source word that is itself a destination (a whole-word shift or rotation of the state reuses
    /// the state's own registers) is staged in a fresh register first.
    pub fn mw_assign(&mut self, dst: &Mw, src: &Mw) {
        assert_eq!(dst.len(), src.len());
        let staged: Vec<LReg> = src
            .0
            .iter()
            .zip(&dst.0)
            .map(|(s, d)| {
                if s != d && dst.0.contains(s) {
                    self.state(*s)
                } else {
                    *s
                }
            })
            .collect();
        for (d, s) in dst.0.iter().zip(&staged) {
            self.assign(*d, *s);
        }
    }

    /// `a` zero-extended or truncated to `n` words.
    pub fn mw_resize(&mut self, a: &Mw, n: usize) -> Mw {
        let z = self.k(0);
        Mw((0..n).map(|i| a.0.get(i).copied().unwrap_or(z)).collect())
    }

    /// `a` sign-extended (from its top bit) or truncated to `n` words.
    pub fn mw_sext(&mut self, a: &Mw, n: usize) -> Mw {
        if n <= a.len() {
            return a.slice(0, n);
        }
        let s = self.mw_top_bit(a);
        let ones = self.k(self.shape.mask());
        let z = self.k(0);
        let fill = self.sel(s, ones, z);
        Mw((0..n)
            .map(|i| a.0.get(i).copied().unwrap_or(fill))
            .collect())
    }

    /// A word value (fits one word) as an `n`-word value.
    pub fn mw_from_word(&mut self, a: LReg, n: usize) -> Mw {
        self.mw_resize(&Mw(vec![a]), n)
    }

    pub fn mw_add(&mut self, a: &Mw, b: &Mw, cin: LReg) -> (Mw, LReg) {
        assert_eq!(a.len(), b.len());
        let mut c = cin;
        let mut out = Vec::with_capacity(a.len());
        for (x, y) in a.0.iter().zip(&b.0) {
            let (s, c2) = self.add(*x, *y, c);
            out.push(s);
            c = c2;
        }
        (Mw(out), c)
    }

    /// `a + b` dropping the carry.
    pub fn mw_plus(&mut self, a: &Mw, b: &Mw) -> Mw {
        let z = self.k(0);
        self.mw_add(a, b, z).0
    }

    pub fn mw_sub(&mut self, a: &Mw, b: &Mw, bin: LReg) -> (Mw, LReg) {
        assert_eq!(a.len(), b.len());
        let mut c = bin;
        let mut out = Vec::with_capacity(a.len());
        for (x, y) in a.0.iter().zip(&b.0) {
            let (d, c2) = self.sub(*x, *y, c);
            out.push(d);
            c = c2;
        }
        (Mw(out), c)
    }

    /// `a − b` dropping the borrow.
    pub fn mw_minus(&mut self, a: &Mw, b: &Mw) -> Mw {
        let z = self.k(0);
        self.mw_sub(a, b, z).0
    }

    /// `0 − a`.
    pub fn mw_neg(&mut self, a: &Mw) -> Mw {
        let zero = self.mw_zero(a.len());
        self.mw_minus(&zero, a)
    }

    /// `a + 1` when `bit` is 1, else `a`, and the carry out.
    pub fn mw_inc(&mut self, a: &Mw, bit: LReg) -> (Mw, LReg) {
        let zero = self.mw_zero(a.len());
        self.mw_add(a, &zero, bit)
    }

    pub fn mw_and(&mut self, a: &Mw, b: &Mw) -> Mw {
        Mw(a.0
            .iter()
            .zip(&b.0)
            .map(|(x, y)| self.and(*x, *y))
            .collect())
    }

    pub fn mw_or(&mut self, a: &Mw, b: &Mw) -> Mw {
        Mw(a.0.iter().zip(&b.0).map(|(x, y)| self.or(*x, *y)).collect())
    }

    pub fn mw_xor(&mut self, a: &Mw, b: &Mw) -> Mw {
        Mw(a.0
            .iter()
            .zip(&b.0)
            .map(|(x, y)| self.xor(*x, *y))
            .collect())
    }

    pub fn mw_not(&mut self, a: &Mw) -> Mw {
        Mw(a.0.iter().map(|x| self.not(*x)).collect())
    }

    pub fn mw_sel(&mut self, c: LReg, t: &Mw, f: &Mw) -> Mw {
        assert_eq!(t.len(), f.len());
        Mw(t.0
            .iter()
            .zip(&f.0)
            .map(|(x, y)| self.sel(c, *x, *y))
            .collect())
    }

    /// A word that is zero exactly when `a` is.
    pub fn mw_or_reduce(&mut self, a: &Mw) -> LReg {
        let mut acc = self.k(0);
        for x in &a.0 {
            acc = self.or(acc, *x);
        }
        acc
    }

    /// 1 when `a` is zero.
    pub fn mw_is_zero(&mut self, a: &Mw) -> LReg {
        let r = self.mw_or_reduce(a);
        let z = self.k(0);
        self.eq(r, z)
    }

    /// 1 when `a == b`.
    pub fn mw_eq(&mut self, a: &Mw, b: &Mw) -> LReg {
        let x = self.mw_xor(a, b);
        self.mw_is_zero(&x)
    }

    /// 1 when `a < b`, both unsigned.
    pub fn mw_ltu(&mut self, a: &Mw, b: &Mw) -> LReg {
        let z = self.k(0);
        let (_, borrow) = self.mw_sub(a, b, z);
        borrow
    }

    /// 1 when `a < b`, both two's complement.
    pub fn mw_lts(&mut self, a: &Mw, b: &Mw) -> LReg {
        let sa = self.mw_top_bit(a);
        let sb = self.mw_top_bit(b);
        let differ = self.xor(sa, sb);
        let u = self.mw_ltu(a, b);
        self.sel(differ, sa, u)
    }

    /// The top bit of `a`, as 0 or 1.
    pub fn mw_top_bit(&mut self, a: &Mw) -> LReg {
        let top = *a.0.last().expect("a nonempty value");
        let w = self.w();
        self.word_bit(top, w - 1)
    }

    /// Bit `i` of `a` (0 past its top).
    pub fn mw_bit(&mut self, a: &Mw, i: u32) -> LReg {
        let w = self.w();
        match a.0.get((i / w) as usize) {
            Some(word) => self.word_bit(*word, i % w),
            None => self.k(0),
        }
    }

    /// `a << s` within `a`'s width.
    pub fn mw_shl(&mut self, a: &Mw, s: u32) -> Mw {
        let w = self.w();
        let n = a.len();
        let (words, bits) = ((s / w) as usize, s % w);
        let z = self.k(0);
        let word = |i: isize| {
            if i < 0 || i as usize >= n {
                z
            } else {
                a.0[i as usize]
            }
        };
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let src = i as isize - words as isize;
            if bits == 0 {
                out.push(word(src));
            } else {
                let hi = word(src);
                let lo = word(src - 1);
                let h = self.shl(hi, bits);
                let l = self.shr(lo, w - bits);
                out.push(self.or(h, l));
            }
        }
        Mw(out)
    }

    /// `a >> s` (logical) within `a`'s width.
    pub fn mw_shr(&mut self, a: &Mw, s: u32) -> Mw {
        let w = self.w();
        let n = a.len();
        let (words, bits) = ((s / w) as usize, s % w);
        let z = self.k(0);
        let word = |i: usize| if i >= n { z } else { a.0[i] };
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let src = i + words;
            if bits == 0 {
                out.push(word(src));
            } else {
                let lo = word(src);
                let hi = word(src + 1);
                let l = self.shr(lo, bits);
                let h = self.shl(hi, w - bits);
                out.push(self.or(l, h));
            }
        }
        Mw(out)
    }

    /// `a >> s` (logical), and 1 when any bit shifted out was set.
    pub fn mw_shr_sticky(&mut self, a: &Mw, s: u32) -> (Mw, LReg) {
        let total = a.len() as u32 * self.w();
        let lost = if s >= total {
            self.mw_or_reduce(a)
        } else {
            let kept_back = self.mw_shl(a, total - s);
            self.mw_or_reduce(&kept_back)
        };
        let sticky = self.nonzero(lost);
        (self.mw_shr(a, s), sticky)
    }

    /// `a >> s` (arithmetic) within `a`'s width.
    pub fn mw_sar(&mut self, a: &Mw, s: u32) -> Mw {
        let n = a.len();
        let wide = self.mw_sext(a, 2 * n);
        let shifted = self.mw_shr(&wide, s);
        shifted.slice(0, n)
    }

    /// `a << amt` for the unsigned amount in `amt`'s low `bits` bits; a shift past the width is zero.
    pub fn mw_shl_var(&mut self, a: &Mw, amt: &Mw, bits: u32) -> Mw {
        let mut cur = a.clone();
        for j in 0..bits {
            let t = self.mw_shl(&cur, 1u32 << j);
            let b = self.mw_bit(amt, j);
            cur = self.mw_sel(b, &t, &cur);
        }
        cur
    }

    /// `a >> amt` (logical) for the amount in `amt`'s low `bits` bits, and the sticky bit of what fell off.
    pub fn mw_shr_var_sticky(&mut self, a: &Mw, amt: &Mw, bits: u32) -> (Mw, LReg) {
        let mut cur = a.clone();
        let mut sticky = self.k(0);
        for j in 0..bits {
            let (t, lost) = self.mw_shr_sticky(&cur, 1u32 << j);
            let b = self.mw_bit(amt, j);
            cur = self.mw_sel(b, &t, &cur);
            let took = self.and(b, lost);
            sticky = self.or(sticky, took);
        }
        (cur, sticky)
    }

    /// `a >> amt` (arithmetic) for the amount in `amt`'s low `bits` bits.
    pub fn mw_sar_var(&mut self, a: &Mw, amt: &Mw, bits: u32) -> Mw {
        let mut cur = a.clone();
        for j in 0..bits {
            let t = self.mw_sar(&cur, 1u32 << j);
            let b = self.mw_bit(amt, j);
            cur = self.mw_sel(b, &t, &cur);
        }
        cur
    }

    /// The leading zeros of `a`, as an `n`-word count (`a`'s width when `a` is zero). `a`'s width must be a
    /// power of two.
    pub fn mw_clz(&mut self, a: &Mw, n: usize) -> Mw {
        let total = a.len() as u32 * self.w();
        assert!(
            total.is_power_of_two(),
            "a leading-zero count over a power-of-two width"
        );
        let mut x = a.clone();
        let mut count = self.mw_zero(n);
        let mut s = total / 2;
        while s >= 1 {
            let top = self.mw_shr(&x, total - s);
            let empty = self.mw_is_zero(&top);
            let shifted = self.mw_shl(&x, s);
            x = self.mw_sel(empty, &shifted, &x);
            let step = self.mw_const(s as u128, n);
            let zero = self.mw_zero(n);
            let add = self.mw_sel(empty, &step, &zero);
            count = self.mw_or(&count, &add);
            s /= 2;
        }
        let top = self.mw_top_bit(&x);
        let missing = self.bit_not(top);
        self.mw_inc(&count, missing).0
    }

    /// The `mul`-bit digits of `a`, least significant first.
    fn digits(&mut self, a: &Mw) -> Vec<LReg> {
        let (w, m) = (self.w(), self.shape.mul);
        let dmask = self.k((1u32 << m) - 1);
        let mut out = Vec::with_capacity(a.len() * (w / m) as usize);
        for x in &a.0 {
            for t in 0..(w / m) {
                let s = self.shr(*x, t * m);
                out.push(self.and(s, dmask));
            }
        }
        out
    }

    /// The full product `a × b` (unsigned), `a.len() + b.len()` words: schoolbook over multiplier-width digits,
    /// each partial product one word, accumulated by result word and then carried.
    pub fn mw_mul(&mut self, a: &Mw, b: &Mw) -> Mw {
        let (w, m) = (self.w(), self.shape.mul);
        let n = a.len() + b.len();
        let da = self.digits(a);
        let db = self.digits(b);
        let mut columns: Vec<Vec<LReg>> = vec![Vec::new(); n + 1];
        for (i, x) in da.iter().enumerate() {
            for (j, y) in db.iter().enumerate() {
                let p = self.mul(*x, *y);
                let at = (i + j) as u32 * m;
                let (k, o) = ((at / w) as usize, at % w);
                if o == 0 {
                    columns[k].push(p);
                } else {
                    let lo = self.shl(p, o);
                    columns[k].push(lo);
                    if o + 2 * m > w {
                        let hi = self.shr(p, w - o);
                        columns[k + 1].push(hi);
                    }
                }
            }
        }
        let most = columns.iter().map(Vec::len).max().unwrap_or(0) as u64;
        let acc_words = self.shape.words_for(w + 64 - (most + 1).leading_zeros());
        let mut result = self.mw_zero(n);
        for (k, col) in columns.iter().enumerate() {
            if col.is_empty() || k >= n {
                continue;
            }
            let mut acc = self.mw_zero(acc_words);
            for x in col {
                let xw = self.mw_from_word(*x, acc_words);
                acc = self.mw_plus(&acc, &xw);
            }
            let placed: Vec<LReg> = (0..n)
                .map(|i| {
                    if i >= k && i - k < acc_words {
                        acc.0[i - k]
                    } else {
                        self.k(0)
                    }
                })
                .collect();
            result = self.mw_plus(&result, &Mw(placed));
        }
        result
    }

    /// Unsigned `a ÷ b` and `a mod b` by restoring division over `a`'s bits; `b` must be nonzero (the caller
    /// guards a zero divisor, where the VM fails).
    pub fn mw_divrem(&mut self, a: &Mw, b: &Mw) -> (Mw, Mw) {
        assert_eq!(a.len(), b.len());
        let n = a.len();
        let bits = n as u32 * self.w();
        let q = self.mw_state(a);
        let zero = self.mw_zero(n + 1);
        let r = self.mw_state(&zero);
        let bw = self.mw_resize(b, n + 1);
        self.repeat(bits, |s| {
            let top = s.mw_top_bit(&q);
            let q2 = s.mw_shl(&q, 1);
            let r_shift = s.mw_shl(&r, 1);
            let low = r_shift.0[0];
            let low2 = s.or(low, top);
            let mut r2 = r_shift.clone();
            r2.0[0] = low2;
            let z = s.k(0);
            let (d, borrow) = s.mw_sub(&r2, &bw, z);
            let fits = s.bit_not(borrow);
            let r3 = s.mw_sel(fits, &d, &r2);
            let q_low = s.or(q2.0[0], fits);
            let mut q3 = q2.clone();
            q3.0[0] = q_low;
            s.mw_assign(&q, &q3);
            s.mw_assign(&r, &r3);
        });
        (q, r.slice(0, n))
    }

    /// The number of set bits of `a`, as an `n`-word count.
    pub fn mw_popcount(&mut self, a: &Mw, n: usize) -> Mw {
        let total = a.len() as u32 * self.w();
        let mut count = self.mw_zero(n);
        for i in 0..total {
            let b = self.mw_bit(a, i);
            count = self.mw_inc(&count, b).0;
        }
        count
    }
}
